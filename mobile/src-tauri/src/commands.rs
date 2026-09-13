//! Everything Shiver's own pages can ask the core to do.

use tauri::{AppHandle, Manager, State};
use tauri_plugin_shiver_push::PushExt;
use uuid::Uuid;

use crate::{
    error::{Error, Result},
    inbox::{self, Inbox},
    login,
    model::{normalize_origin, Folder, MutedChannel, Registry, ServerEntry, ServerInfo, Settings},
    probe,
    store::Store,
    webview::{self, Showing},
};

#[tauri::command]
pub fn list_registry(store: State<'_, Store>) -> Registry {
    store.registry().clone()
}

/// Looks a server up without adding it, so the add screen can show what is about to be added.
#[tauri::command]
pub async fn probe_server(origin: String) -> Result<ServerInfo> {
    let origin = normalize_origin(&origin)?;

    probe::fetch_info(&origin).await
}

/// Adds a server by address, signing in if credentials were given.
///
/// One step, not two. It used to look the server up and then add it, which made the user press a
/// button whose only job was to prove the address worked — `/info` is fetched here anyway, and a
/// server that does not answer it still fails the add. Credentials are optional: leaving them empty
/// adds the server and lets the user sign in on its own page, which is the only thing that works
/// for a server behind an identity provider.
#[tauri::command]
pub async fn add_server(
    app: AppHandle,
    store: State<'_, Store>,
    origin: String,
    identity: Option<String>,
    password: Option<String>,
    // keep the password, so Shiver can sign this server in again by itself when the session expires
    remember_password: Option<bool>,
) -> Result<ServerEntry> {
    let origin = normalize_origin(&origin)?;
    let info = probe::fetch_info(&origin).await?;

    // signed in before the entry exists, so a wrong password fails the add rather than leaving a
    // server in the rail that cannot be opened
    let identity = identity.filter(|value| !value.trim().is_empty());
    let password = password.filter(|value| !value.is_empty());

    let session = match (identity.as_deref(), password.as_deref()) {
        (Some(identity), Some(password)) => {
            Some(login::sign_in(&origin, identity, password).await?)
        }
        _ => None,
    };

    // fetched here, once, rather than linked from the rail: see `ServerEntry::icon_data`
    let icon_data = match info.icon_url.as_deref() {
        Some(url) => probe::fetch_icon(url).await,
        None => None,
    };

    let entry = store.update(|registry| {
        let position = registry
            .servers
            .iter()
            .map(|server| server.position)
            .max()
            .map_or(0, |max| max + 1);

        let entry = ServerEntry {
            id: Uuid::new_v4().to_string(),
            origin: origin.clone(),
            server_id: Some(info.server_id.clone()),
            name: info.name.clone(),
            icon_url: info.icon_url.clone(),
            icon_data: icon_data.clone(),
            // a server starts bounded like any other; trusting one is something the user says
            accept_any_size: false,
            identity: identity.clone(),
            account_label: None,
            folder_id: None,
            position,
        };

        registry.servers.push(entry.clone());

        Ok(entry)
    })?;

    // Kept the moment it is earned. The core can watch this server straight away rather than
    // waiting to read a token out of its page, and the bridge seeds it so the first open lands
    // signed in instead of on the server's connect form.
    if let Some(session) = session {
        inbox::remember_session(&app, &entry.id, &session);

        // Only where the user asked. Sharkord's sessions last a week and cannot be refreshed, so
        // this is the difference between a server Shiver keeps watching and one that goes quiet until
        // it is opened again — and it is their call, because it is their password at rest.
        if remember_password == Some(true) {
            if let Some(password) = password.as_deref() {
                inbox::remember_password(&app, &entry.id, password);
            }
        }
    }

    Ok(entry)
}

#[tauri::command]
pub fn remove_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    inbox::forget_everywhere(&app, &id);
    // the distributor keeps issuing to an endpoint until told otherwise, so a removed server would
    // go on waking the phone about messages Shiver no longer has any business showing
    crate::push::unregister(&app, &id);

    store.update(|registry| {
        let before = registry.servers.len();

        registry.servers.retain(|server| server.id != id);
        registry.muted.retain(|muted| muted.entry_id != id);

        // removing a server can empty the folder it was in
        crate::model::prune_folders(registry);

        if registry.servers.len() == before {
            return Err(Error::UnknownServer);
        }

        if registry.settings.last_server_id.as_deref() == Some(id.as_str()) {
            registry.settings.last_server_id = None;
        }

        Ok(())
    })
}

/// Re-reads a server's public identity and updates the entry.
///
/// The same as desktop's `refresh_server_info`, plus the logo bytes: the mobile rail draws from
/// `icon_data`, so refreshing a name without refreshing that would leave the old icon behind.
#[tauri::command]
pub async fn refresh_server_info(store: State<'_, Store>, id: String) -> Result<ServerEntry> {
    let origin = {
        let registry = store.registry();

        registry
            .servers
            .iter()
            .find(|server| server.id == id)
            .map(|server| server.origin.clone())
            .ok_or(Error::UnknownServer)?
    };

    let info = probe::fetch_info(&origin).await?;

    let icon_data = match info.icon_url.as_deref() {
        Some(url) => probe::fetch_icon(url).await,
        None => None,
    };

    store.update(|registry| {
        let server = registry
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .ok_or(Error::UnknownServer)?;

        server.name = info.name.clone();
        server.icon_url = info.icon_url.clone();
        server.icon_data = icon_data.clone();
        server.server_id = Some(info.server_id.clone());

        Ok(server.clone())
    })
}

/// Marks every channel on a server read.
///
/// Only the server on screen can be marked: Sharkord does this by selecting each channel, which is
/// its own client's job, and on Android only one client is running. Desktop can do it for any
/// server because every server has a webview there. Refused rather than silently ignored, so the
/// menu can say why.
#[tauri::command]
pub async fn mark_server_read(app: AppHandle, id: String) -> Result<()> {
    if app.state::<Showing>().server().as_deref() != Some(id.as_str()) {
        return Err(Error::Webview(
            "Open this server first — Shiver can only mark the server you are looking at as read"
                .into(),
        ));
    }

    let window = webview::main_window(&app)?;

    // exposed by the bridge, which is where the knowledge of Sharkord's store lives
    window
        .eval("window.__SHIVER_MARK_ALL_READ__ && window.__SHIVER_MARK_ALL_READ__()")
        .map_err(|error| Error::Webview(error.to_string()))
}

/// What Shiver knows about being woken while it is closed.
///
/// `distributors` is every app on the phone that can act as a UnifiedPush distributor, which is
/// usually none or one. Nothing here is a secret — an endpoint is not returned, only how many there
/// are — so the settings screen can say what is working without handing the page a capability.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushStatus {
    /// package names of installed distributors
    pub distributors: Vec<String>,
    /// the one Shiver is using, if any
    pub chosen: Option<String>,
    /// servers that have an endpoint, and servers the distributor refused
    pub registered: usize,
    pub failed: usize,
    /// every server, with whether it was chosen and how that is going
    pub servers: Vec<PushServer>,
}

/// One server's row on the notifications screen.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushServer {
    pub id: String,
    pub name: String,
    /// the user asked for this one to be able to wake the phone
    pub wanted: bool,
    /// `off`, `waiting` for the distributor to answer, `ready`, or `failed`
    pub state: &'static str,
}

#[tauri::command]
pub fn push_status(app: AppHandle, store: State<'_, Store>) -> PushStatus {
    let (distributors, chosen) = app.shiver_push().distributors().unwrap_or_default();
    let push = app.state::<crate::push::Push>();
    let (registered, failed) = push.snapshot();
    let registry = store.registry();
    let wanted = registry.settings.push_servers.clone();

    let servers = registry
        .servers
        .iter()
        .map(|server| {
            let chosen_here = wanted.contains(&server.id);

            PushServer {
                id: server.id.clone(),
                name: server.name.clone(),
                wanted: chosen_here,
                state: match () {
                    _ if !chosen_here => "off",
                    _ if push.endpoint(&server.id).is_some() => "ready",
                    _ if push.has_failed(&server.id) => "failed",
                    _ => "waiting",
                },
            }
        })
        .collect();

    PushStatus {
        distributors,
        chosen,
        registered,
        failed,
        servers,
    }
}

/// Turns one server's waking on or off.
#[tauri::command]
pub fn set_push_server(app: AppHandle, entry_id: String, wanted: bool) -> Result<()> {
    crate::push::set_wanted(&app, &entry_id, wanted)
}

/// Chooses a distributor and asks it for an endpoint for each server that has been chosen.
///
/// Only the chosen ones, which on a first run is none: picking a distributor says how Shiver may be
/// woken, not which servers may wake it. The endpoints arrive asynchronously, as broadcasts, so
/// this returns before they exist — the screen listens for `shiver://push` rather than a result.
#[tauri::command]
pub fn set_push_distributor(app: AppHandle, distributor: String) -> Result<()> {
    app.shiver_push()
        .set_distributor(&distributor)
        .map_err(|error| Error::Webview(error.to_string()))?;

    crate::push::register_wanted(&app);

    Ok(())
}

/// Drops everything Shiver is holding to watch servers with: every session, and every kept password.
///
/// The way to say no. Shiver borrows a session from each server it watches so the inbox works from a
/// cold start, and `log_out_server` can only clear the one server whose page is on screen — it
/// clears that page's own sign-in too, which needs the page. This clears Shiver's side for all of
/// them at once, which is what the settings screen offers.
///
/// It leaves each server's own client alone: the auto-login the user set up there is theirs, and
/// signing them out of a server they never asked to leave is not what this button says.
///
/// Notifications and badges stop until the user opens each server again, which is the honest
/// consequence — they came from these sessions.
#[tauri::command]
pub fn forget_sessions(app: AppHandle, store: State<'_, Store>) -> Result<()> {
    let ids: Vec<String> = store
        .registry()
        .servers
        .iter()
        .map(|server| server.id.clone())
        .collect();

    for id in ids {
        inbox::forget_everywhere(&app, &id);
    }

    Ok(())
}

/// Signs out of a server.
///
/// On desktop this clears Shiver's own stored credentials. There are none here, so what it clears is
/// the server's *own* sign-in state — the auto-login token Sharkord persists and the live session —
/// and reloads, which lands the user on that server's connect form. Shiver also drops the token it
/// borrowed, or it would keep a connection open on a session the user just ended.
#[tauri::command]
pub async fn log_out_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    if app.state::<Showing>().server().as_deref() != Some(id.as_str()) {
        return Err(Error::Webview(
            "Open this server first — signing out clears the session held in its own page".into(),
        ));
    }

    inbox::forget_everywhere(&app, &id);

    store.update(|registry| {
        if let Some(server) = registry.servers.iter_mut().find(|server| server.id == id) {
            server.identity = None;
        }

        Ok(())
    })?;

    let window = webview::main_window(&app)?;

    window
        .eval("window.__SHIVER_SIGN_OUT__ && window.__SHIVER_SIGN_OUT__()")
        .map_err(|error| Error::Webview(error.to_string()))
}

/// Unread per server, for the rail on Shiver's own pages.
///
/// The counts come from the core's own connections, so they cover servers with no page on screen —
/// which on mobile is every server but one.
#[tauri::command]
pub fn list_unread(inbox: State<'_, Inbox>) -> std::collections::HashMap<String, u32> {
    inbox.unread()
}

/// Every server's conversations, for Shiver's own direct-message screen.
///
/// The one place the user's conversations across every server are gathered together. It is a
/// command rather than part of the bridge's payload on purpose: only Shiver's own pages have Tauri
/// IPC, so this list can never be read by a server's client — which would otherwise learn the names
/// of everyone the user privately messages on every *other* server they have added.
///
/// The server on screen is missing from this, and correctly so: Shiver holds no connection to it, and
/// its own page draws its own conversations from Sharkord's store.
#[tauri::command]
pub fn list_dms(store: State<'_, Store>, inbox: State<'_, Inbox>) -> Vec<inbox::DmEntry> {
    inbox::collect_dms(&store.registry().servers, &inbox.dms())
}

/// Hands the webview over to a server's own client.
///
/// `dms` is set when the user arrived by tapping the rail's direct-messages tile. Sharkord's dm
/// list is per server and lives in its own sidebar, so Shiver cannot show it itself — it asks the
/// bridge to open it once the server's client is up.
#[tauri::command]
pub async fn open_server(
    app: AppHandle,
    store: State<'_, Store>,
    id: String,
    dms: Option<bool>,
    dm_user: Option<String>,
) -> Result<()> {
    app.state::<Showing>().set_pending_dms(dms.unwrap_or(false));
    app.state::<Showing>().set_pending_dm_user(dm_user);

    let entry = {
        let registry = store.registry();

        registry
            .servers
            .iter()
            .find(|server| server.id == id)
            .cloned()
            .ok_or(Error::UnknownServer)?
    };

    webview::show_server(&app, &entry)?;

    // this server reports for itself from here, so Shiver drops its own socket to it
    inbox::sync(&app);

    store.update(|registry| {
        registry.settings.last_server_id = Some(id.clone());

        Ok(())
    })
}

/// Takes the webview back to Shiver's own pages.
#[tauri::command]
pub async fn show_shiver(app: AppHandle) -> Result<()> {
    webview::show_shiver(&app)?;

    // nothing is on screen now, so every server Shiver has a token for is worth watching
    inbox::sync(&app);

    Ok(())
}

/// Which server the webview is on, so Shiver's pages know what they are returning from.
#[tauri::command]
pub fn showing_server(showing: State<'_, Showing>) -> Option<String> {
    showing.server()
}

/// Signs an existing server in again, and optionally keeps the password for next time.
///
/// The way back from a session Shiver could not renew. Shiver cannot ask a server for a token without
/// credentials, so when a stored session expires and there is no password to fall back on, this is
/// what the user does about it — and if they tick the box, it is the last time they have to.
#[tauri::command]
pub async fn sign_in_server(
    app: AppHandle,
    store: State<'_, Store>,
    id: String,
    identity: String,
    password: String,
    remember_password: Option<bool>,
) -> Result<()> {
    let origin = {
        let registry = store.registry();

        registry
            .servers
            .iter()
            .find(|server| server.id == id)
            .map(|server| server.origin.clone())
            .ok_or(Error::UnknownServer)?
    };

    let session = crate::login::sign_in(&origin, &identity, &password).await?;

    // kept on the entry so Shiver can sign in again without asking who it is signing in as
    store.update(|registry| {
        if let Some(server) = registry.servers.iter_mut().find(|server| server.id == id) {
            server.identity = Some(identity.clone());
        }

        Ok(())
    })?;

    inbox::remember_session(&app, &id, &session);

    if remember_password == Some(true) {
        inbox::remember_password(&app, &id, &password);
    } else {
        inbox::forget_password(&app, &id);
    }

    Ok(())
}

/// Which servers are waiting to be signed in, so Shiver's own pages can say so.
#[tauri::command]
pub fn signed_out_servers(app: AppHandle) -> Vec<String> {
    app.state::<inbox::Inbox>().signed_out()
}

/// Lets one server send Shiver messages of any size, or takes that back.
///
/// The bound Shiver applies by default is about servers nobody here controls; this is the exception
/// for one that somebody does. It takes effect on that server's next connection attempt, which is
/// at most thirty seconds away, so there is nothing to restart.
#[tauri::command]
pub fn set_server_accepts_any_size(
    store: State<'_, Store>,
    entry_id: String,
    accept: bool,
) -> Result<()> {
    store.update(|registry| {
        if let Some(server) = registry
            .servers
            .iter_mut()
            .find(|server| server.id == entry_id)
        {
            server.accept_any_size = accept;
        }

        Ok(())
    })
}

/// Servers Shiver cannot watch, and why.
///
/// Only failures that will not come right on their own end up here — a server sending more in one
/// message than Shiver accepts. A phone off wifi is not a problem to report; it is a phone off wifi.
#[tauri::command]
pub fn watch_problems(app: AppHandle) -> Vec<WatchProblem> {
    app.state::<inbox::Inbox>()
        .problems()
        .into_iter()
        .map(|(entry_id, reason)| WatchProblem { entry_id, reason })
        .collect()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchProblem {
    pub entry_id: String,
    pub reason: String,
}

/// Which servers Shiver can sign in again by itself, so the settings screen can say which are set up.
#[tauri::command]
pub fn remembered_servers(app: AppHandle, store: State<'_, Store>) -> Vec<String> {
    let inbox = app.state::<inbox::Inbox>();
    let registry = store.registry();

    registry
        .servers
        .iter()
        .filter(|server| inbox.has_password(&server.id))
        .map(|server| server.id.clone())
        .collect()
}

/// Which build this is.
///
/// Shiver's own command rather than the `app` plugin's `getVersion`, which needs a capability grant
/// Shiver does not make — it asks for nothing from the plugin acl, and a version string is not worth
/// being the first thing it does.
#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
pub fn get_settings(store: State<'_, Store>) -> Settings {
    store.registry().settings.clone()
}

#[tauri::command]
pub fn update_settings(app: AppHandle, store: State<'_, Store>, settings: Settings) -> Result<()> {
    store.update(|registry| {
        // last_server_id is Shiver's own bookkeeping, not something the settings screen owns
        let last_server_id = registry.settings.last_server_id.clone();

        registry.settings = Settings {
            last_server_id,
            ..settings
        };

        Ok(())
    })?;

    // The webview paints this between pages, so it has to follow the colour the user just picked or
    // the gap between two servers would flash the *old* background instead of no longer flashing.
    // Failure is not worth reporting: the colour they asked for is stored and applied everywhere
    // else, and the cost is a wrong-coloured frame during a navigation.
    if let Ok(window) = webview::main_window(&app) {
        let _ = window
            .set_background_color(Some(webview::background_color(&store.registry().settings)));
    }

    Ok(())
}

/// One row of the rail: a server sitting at the top level, or a folder.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RailRef {
    pub kind: String,
    pub id: String,
}

/// Applies a drag anywhere in the rail.
///
/// Servers and folders share one position space, which is what lets a folder be dragged above a
/// server and the other way round. Servers inside a folder are not part of this ordering and keep
/// their own positions relative to each other.
#[tauri::command]
pub fn reorder_rail(store: State<'_, Store>, ordered: Vec<RailRef>) -> Result<()> {
    store.update(|registry| {
        for (index, item) in ordered.iter().enumerate() {
            let position = index as i32;

            match item.kind.as_str() {
                "folder" => {
                    let folder = registry
                        .folders
                        .iter_mut()
                        .find(|folder| folder.id == item.id)
                        .ok_or(Error::UnknownFolder)?;

                    folder.position = position;
                }
                _ => {
                    let server = registry
                        .servers
                        .iter_mut()
                        .find(|server| server.id == item.id)
                        .ok_or(Error::UnknownServer)?;

                    server.position = position;
                }
            }
        }

        Ok(())
    })
}

/// Creates a folder and moves servers into it in one step, so a drag that makes a folder cannot
/// half-succeed and leave a folder with nothing in it.
#[tauri::command]
pub fn create_folder_with(
    store: State<'_, Store>,
    name: String,
    member_ids: Vec<String>,
) -> Result<Folder> {
    store.update(|registry| {
        // the new folder takes the place of the first server going into it, so it appears where the
        // user dropped rather than at the end of the rail
        let position = registry
            .servers
            .iter()
            .filter(|server| member_ids.contains(&server.id))
            .map(|server| server.position)
            .min()
            .unwrap_or(0);

        let folder = Folder {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.trim().to_string(),
            position,
            expanded: true,
        };

        for (index, id) in member_ids.iter().enumerate() {
            let server = registry
                .servers
                .iter_mut()
                .find(|server| &server.id == id)
                .ok_or(Error::UnknownServer)?;

            server.folder_id = Some(folder.id.clone());
            server.position = index as i32;
        }

        registry.folders.push(folder.clone());

        Ok(folder)
    })
}

/// Moves a server into a folder, or back out to the top level when given nothing.
#[tauri::command]
pub fn set_server_folder(
    store: State<'_, Store>,
    id: String,
    folder_id: Option<String>,
) -> Result<()> {
    store.update(|registry| {
        if let Some(folder_id) = &folder_id {
            if !registry
                .folders
                .iter()
                .any(|folder| &folder.id == folder_id)
            {
                return Err(Error::UnknownFolder);
            }
        }

        let server = registry
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .ok_or(Error::UnknownServer)?;

        server.folder_id = folder_id;

        // taking the second-to-last server out leaves a folder with one, which is not a folder
        crate::model::prune_folders(registry);

        Ok(())
    })
}

#[tauri::command]
pub fn rename_folder(store: State<'_, Store>, id: String, name: String) -> Result<()> {
    store.update(|registry| {
        let folder = registry
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(Error::UnknownFolder)?;

        folder.name = name.trim().to_string();

        Ok(())
    })
}

/// Deleting a folder keeps its servers; they move back to the top level of the rail.
#[tauri::command]
pub fn delete_folder(store: State<'_, Store>, id: String) -> Result<()> {
    store.update(|registry| {
        for server in registry.servers.iter_mut() {
            if server.folder_id.as_deref() == Some(id.as_str()) {
                server.folder_id = None;
            }
        }

        registry.folders.retain(|folder| folder.id != id);

        Ok(())
    })
}

#[tauri::command]
pub fn set_folder_expanded(store: State<'_, Store>, id: String, expanded: bool) -> Result<()> {
    store.update(|registry| {
        let folder = registry
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(Error::UnknownFolder)?;

        folder.expanded = expanded;

        Ok(())
    })
}

/// Applies a drag in the rail. `ordered_ids` is every server in its new order.
///
/// Ids rather than positions, so the caller says what it sees rather than doing arithmetic Shiver
/// would have to trust. A server missing from the list is an error rather than a silent drop: the
/// rail either knows the whole order or it is not the thing that should be setting it.
#[tauri::command]
pub fn reorder_servers(store: State<'_, Store>, ordered_ids: Vec<String>) -> Result<()> {
    store.update(|registry| {
        for (index, id) in ordered_ids.iter().enumerate() {
            let server = registry
                .servers
                .iter_mut()
                .find(|server| &server.id == id)
                .ok_or(Error::UnknownServer)?;

            server.position = index as i32;
        }

        registry.servers.sort_by_key(|server| server.position);

        Ok(())
    })
}

/// Mutes or unmutes one channel on one server.
#[tauri::command]
pub fn set_channel_muted(
    store: State<'_, Store>,
    entry_id: String,
    channel_id: i64,
    muted: bool,
) -> Result<()> {
    store.update(|registry| {
        registry
            .muted
            .retain(|entry| !(entry.entry_id == entry_id && entry.channel_id == channel_id));

        if muted {
            registry.muted.push(MutedChannel {
                entry_id: entry_id.clone(),
                channel_id,
            });
        }

        Ok(())
    })
}
