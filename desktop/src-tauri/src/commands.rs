use tauri::{
    menu::{ContextMenu, MenuBuilder, MenuItemBuilder},
    AppHandle, Emitter, Manager, State,
};

/// Tells Shiver's own webviews the settings changed, so they repaint.
const SETTINGS_EVENT: &str = "shiver://settings";
use uuid::Uuid;

use crate::{
    drain::{self, Readiness},
    error::{Error, Result},
    feed::{DmEntry, Feed, Notification},
    hotkey, jwt, login,
    model::{normalize_origin, Folder, MutedChannel, Registry, ServerEntry, ServerInfo, Settings},
    probe,
    secrets::{self, Secret},
    session::Recovery,
    store::Store,
    voice::{VoiceState, VoiceStatus},
    webviews,
};

/// Forgets the camera and microphone answers WebView2 has stored for every server.
///
/// There is no way back from inside a page: press Block once and the site is refused in silence
/// from then on, which looks like a broken camera rather than a decision. A browser puts the way
/// back in the address bar; Shiver has no address bar, so it has this.
///
/// Answers with how many were forgotten, because "nothing was stored" and "nothing was cleared"
/// are the same button press and the user is owed the difference.
///
/// **Async, and on a blocking thread, and both matter.** WebView2 answers this through a completion
/// handler delivered on the main thread's message loop, and the wait for it is a blocking one. A
/// synchronous `#[tauri::command]` runs on the main thread — so it would be the thing holding up
/// the very loop that has to deliver the answer, which is the deadlock this project already met
/// once when creating webviews (item 3). The visible form was a ten-second hang, "the webview did
/// not answer in time", and the permissions cleared anyway the moment the main thread was let go.
///
/// `spawn_blocking` rather than a bare `async fn` because the wait really does block its thread,
/// and the async runtime's workers are not there to be sat on.
#[tauri::command]
pub async fn reset_media_permissions(app: AppHandle) -> Result<usize> {
    tauri::async_runtime::spawn_blocking(move || crate::permissions::clear_media_permissions(&app))
        .await
        .map_err(|error| Error::Webview(error.to_string()))?
}

#[tauri::command]
pub fn list_registry(store: State<'_, Store>) -> Registry {
    store.registry().clone()
}

/// Looks a server up without adding it, so the add dialog can show what the user is about to add.
#[tauri::command]
pub async fn probe_server(origin: String) -> Result<ServerInfo> {
    let origin = normalize_origin(&origin)?;

    probe::fetch_info(&origin).await
}

/// Adds a server, signing in first when credentials were given.
///
/// Signing in here rather than in the webview is what makes opening a server seamless: Shiver holds
/// the session and seeds it into the page, so the user never meets the login screen. Leaving
/// `identity` empty keeps the old behaviour, which is the fallback for servers behind an identity
/// provider where Shiver cannot do the sign-in itself.
///
/// `remember_password` is the user's choice about the password itself, and it is off unless they
/// say otherwise — the same opt-in mobile has. The session is kept either way: it is what Shiver
/// signs in *with*, it dies in a week on its own, and without it there is no seamless open at all.
/// The password is the thing that outlives everything, and Windows Credential Manager is readable
/// by anything running as that user, so it is not Shiver's to keep uninvited.
#[tauri::command]
pub async fn add_server(
    store: State<'_, Store>,
    origin: String,
    identity: Option<String>,
    password: Option<String>,
    account_label: Option<String>,
    // keep the password, so Shiver can sign this server in again by itself when the session expires
    remember_password: Option<bool>,
) -> Result<ServerEntry> {
    let origin = normalize_origin(&origin)?;
    let info = probe::fetch_info(&origin).await?;

    let credentials = match (identity.as_deref(), password.as_deref()) {
        (Some(identity), Some(password)) if !identity.is_empty() && !password.is_empty() => {
            // proves the credentials before anything is written, so a typo does not leave a
            // half-configured entry in the rail
            let token = login::sign_in(&origin, identity, password).await?;

            Some((identity.to_string(), password.to_string(), token))
        }
        _ => None,
    };

    let entry = store.update(|registry| {
        let position = next_position(registry);

        // the same origin is allowed more than once so a user can hold several accounts on one
        // server, but then the entries need labels to be distinguishable in the rail
        let existing = registry
            .servers
            .iter()
            .filter(|server| server.origin == origin)
            .count();

        // the username already tells two accounts apart, so it is the natural default label
        let label = account_label
            .clone()
            .or_else(|| {
                credentials
                    .as_ref()
                    .map(|(identity, _, _)| identity.clone())
            })
            .or_else(|| (existing > 0).then(|| format!("Account {}", existing + 1)));

        let entry = ServerEntry {
            id: Uuid::new_v4().to_string(),
            origin: origin.clone(),
            server_id: Some(info.server_id.clone()),
            name: info.name.clone(),
            icon_url: info.icon_url.clone(),
            identity: credentials
                .as_ref()
                .map(|(identity, _, _)| identity.clone()),
            account_label: label,
            folder_id: None,
            position,
        };

        registry.servers.push(entry.clone());

        Ok(entry)
    })?;

    if let Some((_, password, token)) = credentials {
        secrets::store(Secret::Session, &entry.id, &token)?;

        if remember_password == Some(true) {
            secrets::store(Secret::Password, &entry.id, &password)?;
        }
    }

    Ok(entry)
}

/// Removes a server and everything Shiver knows about the user on it.
#[tauri::command]
pub async fn remove_server(
    app: AppHandle,
    store: State<'_, Store>,
    feed: State<'_, Feed>,
    id: String,
) -> Result<()> {
    webviews::close_server(&app, &id)?;

    // secrets go before the entry does, so a failure here cannot leave a credential behind for a
    // server that is no longer in the rail
    secrets::forget_all(&id)?;
    feed.forget_entry(&id);

    app.state::<VoiceState>().forget_entry(&id);
    app.state::<Readiness>().forget_entry(&id);
    app.state::<Recovery>().forget_entry(&id);
    app.state::<drain::Openings>().forget_entry(&id);

    store.update(|registry| {
        let before = registry.servers.len();

        registry.servers.retain(|server| server.id != id);
        registry.muted.retain(|muted| muted.entry_id != id);

        if registry.servers.len() == before {
            return Err(Error::UnknownServer);
        }

        if registry.settings.last_server_id.as_deref() == Some(id.as_str()) {
            registry.settings.last_server_id = None;
        }

        Ok(())
    })
}

/// Applies a drag in the rail. `ordered_ids` is the full rail in its new order.
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

/// One row of the rail: either a server sitting at the top level, or a folder.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RailRef {
    pub kind: String,
    pub id: String,
}

/// Applies a drag anywhere in the rail.
///
/// Servers and folders share one position space, which is what lets a folder be dragged above a
/// server and vice versa. Servers inside a folder are not part of this ordering and keep their own
/// positions relative to each other.
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
/// leave an empty one behind if it fails halfway.
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
            .unwrap_or_else(|| next_position(registry));

        let folder = Folder {
            id: Uuid::new_v4().to_string(),
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

/// The folder's own context menu.
#[tauri::command]
pub async fn show_folder_menu(app: AppHandle, id: String) -> Result<()> {
    let window = webviews::main_window(&app)?;

    let rename =
        MenuItemBuilder::with_id(format!("rename-folder:{id}"), "Rename folder").build(&app)?;
    let delete =
        MenuItemBuilder::with_id(format!("delete-folder:{id}"), "Delete folder").build(&app)?;

    let menu = MenuBuilder::new(&app)
        .item(&rename)
        .separator()
        .item(&delete)
        .build()?;

    menu.popup(window)?;

    Ok(())
}

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

        Ok(())
    })
}

#[tauri::command]
pub fn create_folder(store: State<'_, Store>, name: String) -> Result<Folder> {
    store.update(|registry| {
        let folder = Folder {
            id: Uuid::new_v4().to_string(),
            name: name.trim().to_string(),
            position: registry.folders.len() as i32,
            expanded: true,
        };

        registry.folders.push(folder.clone());

        Ok(folder)
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

/// Deleting a folder keeps its servers, they just move back to the top level of the rail.
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

/// Shows a server, creating its webview on first use, and remembers it for the next launch.
///
/// Async on purpose. Creating or moving a webview dispatches to the event loop and blocks until it
/// answers, and tauri runs synchronous commands on the main thread, which *is* that loop: doing
/// this from a `fn` deadlocks the app the first time a server is opened.
#[tauri::command]
pub async fn select_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    let (entry, settings, muted) = {
        let registry = store.registry();

        let entry = registry
            .servers
            .iter()
            .find(|server| server.id == id)
            .cloned()
            .ok_or(Error::UnknownServer)?;

        let muted = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == id)
            .map(|muted| muted.channel_id)
            .collect::<Vec<_>>();

        (entry, registry.settings.clone(), muted)
    };

    let token = ensure_session(&entry).await;
    let locked = voice_locked_for(&app, &id);

    webviews::show_server(&app, &entry, &settings, token.as_deref(), &muted, locked)?;

    // deliberately does *not* mark the whole server read. The channel the page lands on clears
    // itself once the drain sees it on screen, and what is left is the point: the badge goes on
    // showing the unread in the channels the user has not looked at. "Mark all as read" in the
    // rail's menu is the bulk action.

    store.update(|registry| {
        registry.settings.last_server_id = Some(id.clone());

        Ok(())
    })
}

/// Whether a page must refuse to start a call, because another entry is already holding one.
fn voice_locked_for(app: &AppHandle, entry_id: &str) -> bool {
    app.state::<VoiceState>()
        .holder()
        .is_some_and(|holder| holder != entry_id)
}

/// Brings a server up without showing it, and says whether its client is already connected.
///
/// This is what lets Shiver cover a connecting server with its own cached messages rather than with
/// Sharkord's loading state. The shell calls this first: on `true` it goes straight to
/// `select_server`, and on `false` it draws the wait itself until `shiver://server-ready` arrives.
#[tauri::command]
pub async fn prepare_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<bool> {
    let (entry, settings, muted) = {
        let registry = store.registry();

        let entry = registry
            .servers
            .iter()
            .find(|server| server.id == id)
            .cloned()
            .ok_or(Error::UnknownServer)?;

        let muted = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == id)
            .map(|muted| muted.channel_id)
            .collect::<Vec<_>>();

        (entry, registry.settings.clone(), muted)
    };

    // usually already up: every server is opened at launch. this covers the one being opened
    // before the preload reached it, and a server added since.
    if app.get_webview(&webviews::webview_label(&id)).is_none() {
        let token = ensure_session(&entry).await;
        let locked = voice_locked_for(&app, &id);

        webviews::preload_server(&app, &entry, &settings, token.as_deref(), &muted, locked)?;
    }

    Ok(app.state::<Readiness>().is_ready(&id))
}

/// The call Shiver is showing in the rail, wherever it is running.
#[tauri::command]
pub fn voice_status(voice: State<'_, VoiceState>) -> Option<VoiceStatus> {
    voice.status()
}

/// Works the call's own controls from Shiver's chrome, whichever server is on screen.
///
/// It always acts on the entry holding the call rather than on the visible one, which is the whole
/// point: the controls have to reach a call running on a server the user has navigated away from.
#[tauri::command]
pub async fn voice_control(app: AppHandle, action: String) -> Result<()> {
    if !matches!(action.as_str(), "mic" | "sound" | "leave") {
        return Err(Error::Webview(format!("Unknown voice action '{action}'")));
    }

    let Some(holder) = app.state::<VoiceState>().holder() else {
        return Ok(());
    };

    webviews::run_voice_action(&app, &holder, &action);

    Ok(())
}

/// Hides the active server so a Shiver panel can use the whole window.
#[tauri::command]
pub async fn show_shell(app: AppHandle) -> Result<()> {
    webviews::show_shell_only(&app)
}

/// Leaves the DM inbox. The conversation views stay up, hidden, so the next one opens instantly.
#[tauri::command]
pub async fn exit_dm_split(app: AppHandle) -> Result<()> {
    webviews::hide_dm_views(&app)
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
pub async fn update_settings(
    app: AppHandle,
    store: State<'_, Store>,
    settings: Settings,
) -> Result<()> {
    let saved = store.update(|registry| {
        // last_server_id is Shiver's own bookkeeping, not something the settings screen owns
        let last_server_id = registry.settings.last_server_id.clone();

        registry.settings = Settings {
            last_server_id,
            ..settings
        };

        Ok(registry.settings.clone())
    })?;

    // re-registered rather than diffed: the shortcut may be unchanged, but the user may equally
    // have just cleared it or taken it from another application
    hotkey::apply(&app, saved.mute_hotkey.as_deref());

    // every page repaints in place: the server clients through the bridge, and Shiver's own bell and
    // popup through an event, since each is a separate webview that would otherwise keep the old
    // colours until it was next created
    webviews::push_theme(&app, &saved);

    let _ = app.emit_to(webviews::SHELL_WEBVIEW, SETTINGS_EVENT, ());
    let _ = app.emit_to(webviews::OVERLAY_WEBVIEW, SETTINGS_EVENT, ());
    let _ = app.emit_to(webviews::POPUP_WEBVIEW, SETTINGS_EVENT, ());

    Ok(())
}

/// Re-reads a server's name and logo, so a rename on the server shows up in the rail.
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

    store.update(|registry| {
        let server = registry
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .ok_or(Error::UnknownServer)?;

        server.name = info.name.clone();
        server.icon_url = info.icon_url.clone();
        server.server_id = Some(info.server_id.clone());

        Ok(server.clone())
    })
}

/// Returns a session token that will still be valid for a while, signing in again if the stored one
/// is missing or close to expiring.
///
/// Never fails the caller: if Shiver cannot get a session, the server's own login page is a perfectly
/// good fallback and is what the user would otherwise have seen anyway.
pub(crate) async fn ensure_session(entry: &ServerEntry) -> Option<String> {
    let stored = secrets::read(Secret::Session, &entry.id).ok().flatten();

    if let Some(token) = &stored {
        if !jwt::needs_refresh(token) {
            return stored;
        }
    }

    let identity = entry.identity.as_deref()?;
    let password = secrets::read(Secret::Password, &entry.id).ok().flatten()?;

    match login::sign_in(&entry.origin, identity, &password).await {
        Ok(token) => {
            let _ = secrets::store(Secret::Session, &entry.id, &token);

            Some(token)
        }
        Err(error) => {
            eprintln!(
                "[shiver] could not refresh the session for {}: {error}",
                entry.origin
            );

            // an expired token is worse than none: the client would try it, fail, and clear its own
            // auto-login state, so hand back nothing and let the login page do its job
            stored.filter(|token| !jwt::needs_refresh(token))
        }
    }
}

/// Signs a server out, and leaves it signed out.
///
/// The session *and* the password go. Keeping the password would mean Shiver signing straight back in
/// — either through `ensure_session` on the next open or through `session.rs` the moment the page
/// showed its login form — so a log out that kept it would not be one. The username goes with them,
/// which puts the entry into the same state as a server added without credentials: still in the
/// rail, still openable, and asking the user who they are.
#[tauri::command]
pub async fn log_out_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    secrets::forget_all(&id)?;

    let entry = store.update(|registry| {
        let server = registry
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .ok_or(Error::UnknownServer)?;

        server.identity = None;

        Ok(server.clone())
    })?;

    // whatever Shiver was doing about this server's session, it is not doing it any more
    app.state::<Recovery>().forget_entry(&id);
    app.state::<Readiness>().forget_entry(&id);

    let settings = store.registry().settings.clone();

    // rebuilt with no token, so the bridge seeds nothing and clears what it seeded before
    webviews::close_server(&app, &id)?;
    webviews::close_dm_view(&app, &id);

    let locked = voice_locked_for(&app, &id);

    webviews::show_server(&app, &entry, &settings, None, &[], locked)?;
    webviews::preload_dm_view(&app, &entry, &settings, None)?;

    Ok(())
}

/// Signs an existing entry in, for a server that was added without credentials or whose password
/// changed.
///
/// `remember_password` is asked again here rather than remembered from the add, so the answer can
/// change: a server signed in once without keeping the password can be told to keep it, and one
/// that kept it can have it dropped by unticking the box.
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

    let token = login::sign_in(&origin, &identity, &password).await?;

    secrets::store(Secret::Session, &id, &token)?;

    // unticked drops whatever was kept before, so the box says what is actually stored rather than
    // only what happens from now on
    if remember_password == Some(true) {
        secrets::store(Secret::Password, &id, &password)?;
    } else {
        secrets::forget(Secret::Password, &id)?;
    }

    store.update(|registry| {
        let server = registry
            .servers
            .iter_mut()
            .find(|server| server.id == id)
            .ok_or(Error::UnknownServer)?;

        server.identity = Some(identity.clone());

        if server.account_label.is_none() {
            server.account_label = Some(identity.clone());
        }

        Ok(())
    })?;

    // the page has to be rebuilt for the seeded session to take effect
    webviews::close_server(&app, &id)?;
    webviews::close_dm_view(&app, &id);
    app.state::<Readiness>().forget_entry(&id);

    // the user has given Shiver a password that works, so whatever it failed at before is history
    app.state::<Recovery>().forget_entry(&id);

    // the shell opens it from here, so the rebuilt page comes up behind Shiver's connecting screen
    // rather than the user watching the client boot
    Ok(())
}

/// Opens the rail's context menu as a real OS menu.
///
/// It has to be native: the rail is drawn by the shell webview, which sits *underneath* the server
/// webview, so an html menu is painted over the moment it extends past the rail. A native menu is
/// not part of any webview and draws above all of them.
#[tauri::command]
pub async fn show_server_menu(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    let window = webviews::main_window(&app)?;

    let (in_folder, signed_in) = {
        let registry = store.registry();
        let server = registry.servers.iter().find(|server| server.id == id);

        (
            server.is_some_and(|server| server.folder_id.is_some()),
            // a stored username is what says Shiver holds this server's session rather than the
            // user having signed in on its own page
            server.is_some_and(|server| server.identity.is_some()),
        )
    };

    // the id travels in the menu item id, so the handler needs no state of its own
    let open = MenuItemBuilder::with_id(format!("open:{id}"), "Open").build(&app)?;
    let mark_read =
        MenuItemBuilder::with_id(format!("markread:{id}"), "Mark all as read").build(&app)?;
    let session = if signed_in {
        MenuItemBuilder::with_id(format!("logout:{id}"), "Log out").build(&app)?
    } else {
        MenuItemBuilder::with_id(format!("signin:{id}"), "Sign in").build(&app)?
    };
    let refresh =
        MenuItemBuilder::with_id(format!("refresh:{id}"), "Refresh name and icon").build(&app)?;
    let remove =
        MenuItemBuilder::with_id(format!("remove:{id}"), "Remove from Shiver").build(&app)?;

    let mut builder = MenuBuilder::new(&app).items(&[&open, &mark_read, &refresh]);

    if in_folder {
        let take_out =
            MenuItemBuilder::with_id(format!("unfolder:{id}"), "Move out of folder").build(&app)?;

        builder = builder.separator().item(&take_out);
    }

    let menu = builder.separator().item(&session).item(&remove).build()?;

    menu.popup(window)?;

    Ok(())
}

/// Opens or closes the notification feed under the bell. Returns the new state, so the bell can
/// show itself as active without tracking it separately.
#[tauri::command]
pub async fn toggle_popup(app: AppHandle) -> Result<bool> {
    let open = app.state::<webviews::ActiveServer>().should_open_popup();

    webviews::set_popup_open(&app, open)?;

    Ok(open)
}

/// Closes the feed, for the popup's own close button.
#[tauri::command]
pub async fn close_popup(app: AppHandle) -> Result<()> {
    webviews::set_popup_open(&app, false)
}

/// Closes the feed because the user clicked away from it.
///
/// Separate from `close_popup` because only this arms the reopen guard. The close button is a
/// deliberate close with focus still inside the feed, and needs no guard; clicking away is the one
/// that races the bell.
#[tauri::command]
pub async fn dismiss_popup(app: AppHandle) -> Result<()> {
    app.state::<webviews::ActiveServer>().note_popup_dismissed();

    webviews::set_popup_open(&app, false)
}

/// Opens one conversation *inside* Shiver's DM window.
///
/// Shiver's cross-server list keeps the left 288px and the conversation is rendered beside it by a
/// webview of its own, dedicated to DMs, with its own sidebar hidden. Nothing about the conversation
/// is reimplemented: it is still Sharkord drawing it.
///
/// The dedicated webview is the point. Driving the server view instead changed where that view was
/// sitting (DM mode and selected channel are page state), so going back to the server landed on the
/// conversation rather than the channel the user had left.
#[tauri::command]
pub async fn open_dm(
    app: AppHandle,
    store: State<'_, Store>,
    entry_id: String,
    name: String,
) -> Result<()> {
    let (entry, settings) = {
        let registry = store.registry();

        let entry = registry
            .servers
            .iter()
            .find(|server| server.id == entry_id)
            .cloned()
            .ok_or(Error::UnknownServer)?;

        (entry, registry.settings.clone())
    };

    let token = ensure_session(&entry).await;
    let created = webviews::show_dm_view(&app, &entry, &settings, token.as_deref(), &name)?;

    // a page created just now already has the conversation in its config and opens it itself; only
    // an existing page needs telling
    if created {
        return Ok(());
    }

    let Some(webview) = app.get_webview(&webviews::dm_webview_label(&entry_id)) else {
        return Ok(());
    };

    let payload = serde_json::to_string(&name).unwrap_or_else(|_| "\"\"".into());

    // the page may still be connecting, so the call retries briefly on its own side
    webview
        .eval(format!(
            "window.__SHIVER_OPEN_DM__ && window.__SHIVER_OPEN_DM__({payload})"
        ))
        .map_err(|error| Error::Webview(error.to_string()))?;

    Ok(())
}

#[tauri::command]
pub fn list_notifications(feed: State<'_, Feed>) -> Vec<Notification> {
    feed.notifications()
}

#[tauri::command]
pub fn list_dms(feed: State<'_, Feed>) -> Vec<DmEntry> {
    feed.dms()
}

#[tauri::command]
pub fn unread_count(feed: State<'_, Feed>) -> usize {
    feed.unread_count()
}

/// Unread per rail entry, for the badges on the server icons.
#[tauri::command]
pub fn unread_counts(feed: State<'_, Feed>) -> std::collections::HashMap<String, usize> {
    feed.unread_by_entry()
}

/// Marks one whole server read, from the rail's context menu.
///
/// Two things are unread about a server and both are settled here: Shiver's own notifications, which
/// is what its rail badge counts, and Sharkord's per-channel unread, which is what the server's own
/// channel list shows. Doing only the first would clear the badge and leave every channel still
/// bold, which would read as the menu item not having worked.
#[tauri::command]
pub async fn mark_server_read(app: AppHandle, entry_id: String) -> Result<()> {
    webviews::mark_all_read(&app, &entry_id);

    if app.state::<Feed>().mark_entry_read(&entry_id) {
        drain::notify_feed_changed(&app);
    }

    Ok(())
}

#[tauri::command]
pub fn mark_notifications_read(app: AppHandle, feed: State<'_, Feed>) {
    feed.mark_all_read();

    // the rail draws a badge per server and the bell draws a total, and both are other webviews:
    // without this they keep showing what was unread before the feed was read, until some later
    // change happens to emit the event for them
    drain::notify_feed_changed(&app);
}

#[tauri::command]
pub fn clear_notifications(app: AppHandle, feed: State<'_, Feed>) {
    feed.clear();

    drain::notify_feed_changed(&app);
}

/// Mutes or unmutes one channel on one rail entry, and tells that page to redraw its channel list.
#[tauri::command]
pub fn set_channel_muted(
    app: AppHandle,
    store: State<'_, Store>,
    entry_id: String,
    channel_id: i64,
    muted: bool,
) -> Result<()> {
    let remaining = store.update(|registry| {
        registry
            .muted
            .retain(|entry| !(entry.entry_id == entry_id && entry.channel_id == channel_id));

        if muted {
            registry.muted.push(MutedChannel {
                entry_id: entry_id.clone(),
                channel_id,
            });
        }

        Ok(registry
            .muted
            .iter()
            .filter(|entry| entry.entry_id == entry_id)
            .map(|entry| entry.channel_id)
            .collect::<Vec<_>>())
    })?;

    webviews::push_muted(&app, &entry_id, &remaining);

    Ok(())
}

/// Brings every server in the rail online at launch, whether or not the user opens it.
///
/// Each gets a hidden webview running the real client, so notifications, DMs and mutes flow from
/// all of them at once rather than only from the one being looked at. Servers are done one at a
/// time: signing in is a network round trip each, and starting a dozen clients simultaneously would
/// make the window that much slower to become usable.
pub(crate) async fn preload_all_servers(app: AppHandle) {
    let (entries, settings) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        (registry.servers.clone(), registry.settings.clone())
    };

    let total = entries.len();
    let mut online = 0;
    let mut sessions = Vec::with_capacity(total);

    for entry in entries {
        let token = ensure_session(&entry).await;

        let muted = {
            let store = app.state::<Store>();
            let registry = store.registry();

            registry
                .muted
                .iter()
                .filter(|muted| muted.entry_id == entry.id)
                .map(|muted| muted.channel_id)
                .collect::<Vec<_>>()
        };

        let locked = voice_locked_for(&app, &entry.id);

        match webviews::preload_server(&app, &entry, &settings, token.as_deref(), &muted, locked) {
            Ok(()) => online += 1,
            // one unreachable server must not stop the rest from coming up
            Err(error) => eprintln!("[shiver] could not preload {}: {error}", entry.origin),
        }

        sessions.push((entry, token));
    }

    if total > 0 {
        eprintln!("[shiver] {online}/{total} servers online");
    }

    // the conversation views come second, on the session each server just established. the rail is
    // what the user is waiting for, so nothing here is allowed to delay a server coming up.
    let mut conversations = 0;

    for (entry, token) in &sessions {
        match webviews::preload_dm_view(&app, entry, &settings, token.as_deref()) {
            Ok(()) => conversations += 1,
            Err(error) => eprintln!(
                "[shiver] could not preload the conversation view for {}: {error}",
                entry.origin
            ),
        }
    }

    if total > 0 {
        eprintln!("[shiver] {conversations}/{total} conversation views ready");
    }
}

fn next_position(registry: &Registry) -> i32 {
    registry
        .servers
        .iter()
        .map(|server| server.position)
        .max()
        .map_or(0, |max| max + 1)
}
