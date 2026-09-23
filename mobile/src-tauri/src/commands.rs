//! Commands called from Shiver's own pages. Server pages have no IPC.
//!
//! Commands that write the registry are `async`, so the disk write happens off the UI thread.
//! Passwords arrive as `Zeroizing<String>` so Shiver's copies are wiped.

use tauri::{AppHandle, Manager, State};
use tauri_plugin_shiver_push::PushExt;
use uuid::Uuid;
use zeroize::Zeroizing;

use shiver_core::{login, probe};

use crate::{
    error::{Error, Result},
    inbox::{self, Inbox},
    model::{normalize_origin, Folder, Registry, ServerEntry, ServerInfo, Settings},
    store::{RegistryStore, Store},
    webview::{self, Showing},
};

type Password = Zeroizing<String>;

const MAX_FOLDER_NAME: usize = 100;

fn entry_of(store: &Store, id: &str) -> Result<ServerEntry> {
    store
        .registry()
        .server(id)
        .cloned()
        .ok_or(Error::UnknownServer)
}

fn clamp_name(name: &str) -> Result<String> {
    let trimmed = name.trim();

    if trimmed.is_empty() {
        return Err(Error::InvalidInput("A folder needs a name".into()));
    }

    Ok(trimmed.chars().take(MAX_FOLDER_NAME).collect())
}

fn require_showing(app: &AppHandle, id: &str, why: &str) -> Result<()> {
    if app.state::<Showing>().server().as_deref() == Some(id) {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "Open this server first — {why}"
        )))
    }
}

fn eval_in_page(app: &AppHandle, script: &str) -> Result<()> {
    Ok(webview::main_window(app)?.eval(script)?)
}

#[tauri::command]
pub fn list_registry(store: State<'_, Store>) -> Registry {
    Registry::clone(&store.registry())
}

#[tauri::command]
pub async fn probe_server(origin: String) -> Result<ServerInfo> {
    Ok(probe::fetch_info(&normalize_origin(&origin)?).await?)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCheck {
    #[serde(flatten)]
    pub info: ServerInfo,
    /// Absent when unchecked (no credentials); `None` when checked and not installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<Option<String>>,
}

/// `/info`, plus — given credentials — whether the companion plugin is installed (only a signed-in
/// join reveals it). Nothing is stored.
#[tauri::command]
pub async fn check_server(
    origin: String,
    identity: Option<String>,
    password: Option<Password>,
) -> Result<ServerCheck> {
    let origin = normalize_origin(&origin)?;
    let info = probe::fetch_info(&origin).await?;

    let (Some(identity), Some(password)) = (
        identity.filter(|identity| !identity.trim().is_empty()),
        password.filter(|password| !password.is_empty()),
    ) else {
        return Ok(ServerCheck { info, plugin: None });
    };

    let token = Zeroizing::new(login::sign_in(&origin, identity.trim(), &password).await?);
    let session = crate::sharkord::open(&origin, &token, false)
        .await
        .map_err(|error| Error::Unreachable(format!("{origin}: {error}")))?;

    Ok(ServerCheck {
        info,
        plugin: Some(session.joined.plugin_version.clone()),
    })
}

/// Adds a server, signing in first when credentials are given (so a wrong password adds nothing).
/// The password is kept unless `remember_password` is `false`.
#[tauri::command]
pub async fn add_server(
    app: AppHandle,
    store: State<'_, Store>,
    origin: String,
    identity: Option<String>,
    password: Option<Password>,
    remember_password: Option<bool>,
) -> Result<ServerEntry> {
    let origin = normalize_origin(&origin)?;
    let info = probe::fetch_info(&origin).await?;
    let identity = identity
        .map(|identity| identity.trim().to_string())
        .filter(|identity| !identity.is_empty());
    let password = password.filter(|password| !password.is_empty());

    let session = match (&identity, &password) {
        (Some(identity), Some(password)) => Some(Zeroizing::new(
            login::sign_in(&origin, identity, password).await?,
        )),
        _ => None,
    };

    // inlined rather than linked: the rail is drawn in other servers' pages
    let icon_data = match info.icon_url.as_deref() {
        Some(url) => probe::fetch_icon(url).await,
        None => None,
    };

    let entry = store.update(|registry| {
        let entry = ServerEntry {
            id: Uuid::new_v4().to_string(),
            origin: origin.clone(),
            server_id: Some(info.server_id.clone()),
            name: info.name.clone(),
            icon_url: info.icon_url.clone(),
            icon_data: icon_data.clone(),
            identity: identity.clone().filter(|_| session.is_some()),
            position: registry.next_position(),
            push_token: Some(Uuid::new_v4().simple().to_string()),
            ..Default::default()
        };

        registry.servers.push(entry.clone());

        Ok(entry)
    })?;

    if let Some(session) = &session {
        inbox::remember_session(&app, &entry.id, session);

        if let (true, Some(password)) = (remember_password != Some(false), &password) {
            inbox::remember_password(&app, &entry.id, password);
        }
    }

    Ok(entry)
}

/// Takes back the stored password (the session stays until it expires).
#[tauri::command]
pub fn forget_password(app: AppHandle, id: String) {
    inbox::forget_password(&app, &id);
}

/// Removes a server and everything Shiver keeps for it.
#[tauri::command]
pub async fn remove_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    entry_of(&store, &id)?;

    // before the entry goes: unregistering needs its push token
    crate::push::unregister(&app, &id);
    inbox::forget_everywhere(&app, &id);

    store.update(|registry| {
        registry.servers.retain(|server| server.id != id);
        registry.muted.retain(|muted| muted.entry_id != id);
        registry
            .settings
            .push_servers
            .retain(|pushed| pushed != &id);
        crate::model::prune_folders(registry);

        if registry.settings.last_server_id.as_deref() == Some(id.as_str()) {
            registry.settings.last_server_id = None;
        }

        Ok(())
    })
}

/// Re-reads a server's name and logo (including the inlined logo bytes).
#[tauri::command]
pub async fn refresh_server_info(store: State<'_, Store>, id: String) -> Result<ServerEntry> {
    let info = probe::fetch_info(&entry_of(&store, &id)?.origin).await?;
    let icon_data = match info.icon_url.as_deref() {
        Some(url) => probe::fetch_icon(url).await,
        None => None,
    };

    store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Error::UnknownServer)?;

        server.name = info.name.clone();
        server.icon_url = info.icon_url.clone();
        server.icon_data = icon_data.clone();
        server.server_id = Some(info.server_id.clone());

        Ok(server.clone())
    })
}

/// Marks every channel read. Only the server on screen can do this (its own client does the work).
#[tauri::command]
pub async fn mark_server_read(app: AppHandle, id: String) -> Result<()> {
    require_showing(
        &app,
        &id,
        "Shiver can only mark the server you are looking at as read",
    )?;
    eval_in_page(
        &app,
        "window.__SHIVER_MARK_ALL_READ__ && window.__SHIVER_MARK_ALL_READ__()",
    )
}

/* ── push ── */

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushStatus {
    /// package names of installed distributors
    pub distributors: Vec<String>,
    pub chosen: Option<String>,
    pub registered: usize,
    pub failed: usize,
    pub servers: Vec<PushServer>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushServer {
    pub id: String,
    pub name: String,
    pub wanted: bool,
    /// `off`, `waiting`, `ready` or `failed`
    pub state: &'static str,
}

/// What the notifications screen shows. No endpoint is returned, only whether one exists.
#[tauri::command]
pub fn push_status(app: AppHandle, store: State<'_, Store>) -> PushStatus {
    let (distributors, chosen) = app.shiver_push().distributors().unwrap_or_default();
    let push = app.state::<crate::push::Push>();
    let (registered, failed) = push.snapshot();
    let registry = store.registry();

    let servers = registry
        .servers
        .iter()
        .map(|server| {
            let wanted = registry.settings.push_servers.contains(&server.id);

            PushServer {
                id: server.id.clone(),
                name: server.name.clone(),
                wanted,
                state: if !wanted {
                    "off"
                } else if push.endpoint(&server.id).is_some() {
                    "ready"
                } else if push.has_failed(&server.id) {
                    "failed"
                } else {
                    "waiting"
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

#[tauri::command]
pub async fn set_push_server(app: AppHandle, entry_id: String, wanted: bool) -> Result<()> {
    crate::push::set_wanted(&app, &entry_id, wanted)
}

/// Chooses a distributor and registers the chosen servers with it (answers arrive as events).
#[tauri::command]
pub fn set_push_distributor(app: AppHandle, distributor: String) -> Result<()> {
    app.shiver_push()
        .set_distributor(&distributor)
        .map_err(|error| Error::Webview(error.to_string()))?;

    crate::push::register_wanted(&app);

    Ok(())
}

/* ── sessions ── */

/// Drops every session and stored password Shiver holds (each server's own page sign-in stays).
#[tauri::command]
pub fn forget_sessions(app: AppHandle, store: State<'_, Store>) {
    let ids: Vec<String> = store
        .registry()
        .servers
        .iter()
        .map(|server| server.id.clone())
        .collect();

    for id in ids {
        inbox::forget_everywhere(&app, &id);
    }
}

/// Signs out of the server on screen: Shiver's session and password for it, its identity, and the
/// page's own sign-in state.
#[tauri::command]
pub async fn log_out_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    require_showing(
        &app,
        &id,
        "signing out clears the session held in its own page",
    )?;

    inbox::forget_everywhere(&app, &id);

    store.update(|registry| {
        registry
            .server_mut(&id)
            .ok_or(Error::UnknownServer)?
            .identity = None;

        Ok(())
    })?;

    eval_in_page(
        &app,
        "window.__SHIVER_SIGN_OUT__ && window.__SHIVER_SIGN_OUT__()",
    )
}

/// Signs an existing server in (after an expired session with no password, or a changed one).
#[tauri::command]
pub async fn sign_in_server(
    app: AppHandle,
    store: State<'_, Store>,
    id: String,
    identity: String,
    password: Password,
    remember_password: Option<bool>,
) -> Result<()> {
    let origin = entry_of(&store, &id)?.origin;
    let identity = identity.trim().to_string();
    let session = Zeroizing::new(login::sign_in(&origin, &identity, &password).await?);

    store.update(|registry| {
        registry
            .server_mut(&id)
            .ok_or(Error::UnknownServer)?
            .identity = Some(identity.clone());

        Ok(())
    })?;

    inbox::remember_session(&app, &id, &session);

    if remember_password != Some(false) {
        inbox::remember_password(&app, &id, &password);
    } else {
        inbox::forget_password(&app, &id);
    }

    Ok(())
}

#[tauri::command]
pub fn signed_out_servers(inbox: State<'_, Inbox>) -> Vec<String> {
    inbox.signed_out()
}

/// Entries with a stored password.
#[tauri::command]
pub fn remembered_servers(inbox: State<'_, Inbox>, store: State<'_, Store>) -> Vec<String> {
    store
        .registry()
        .servers
        .iter()
        .filter(|server| inbox.has_password(&server.id))
        .map(|server| server.id.clone())
        .collect()
}

/// Raises (or restores) the message size limit; takes effect on the next connection attempt.
#[tauri::command]
pub async fn set_accept_any_size(store: State<'_, Store>, id: String, accept: bool) -> Result<()> {
    store.update(|registry| {
        registry
            .server_mut(&id)
            .ok_or(Error::UnknownServer)?
            .accept_any_size = accept;

        Ok(())
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchProblem {
    pub entry_id: String,
    pub reason: String,
}

/// Servers Shiver cannot watch, and why (only failures that will not fix themselves).
#[tauri::command]
pub fn watch_problems(inbox: State<'_, Inbox>) -> Vec<WatchProblem> {
    inbox
        .problems()
        .into_iter()
        .map(|(entry_id, reason)| WatchProblem { entry_id, reason })
        .collect()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatus {
    pub entry_id: String,
    /// `None`: connected, and no plugin. Servers not yet connected are absent.
    pub version: Option<String>,
}

#[tauri::command]
pub fn server_plugins(inbox: State<'_, Inbox>) -> Vec<PluginStatus> {
    inbox
        .plugins()
        .into_iter()
        .map(|(entry_id, version)| PluginStatus { entry_id, version })
        .collect()
}

/* ── the rail and the inbox ── */

#[tauri::command]
pub fn unread_counts(inbox: State<'_, Inbox>) -> std::collections::HashMap<String, u32> {
    inbox.unread()
}

/// Every server's conversations. Only Shiver's own pages can call this, so no server learns who the
/// user talks to on the others.
#[tauri::command]
pub fn list_dms(store: State<'_, Store>, inbox: State<'_, Inbox>) -> Vec<inbox::DmEntry> {
    inbox::collect_dms(&store.registry().servers, &inbox.dms())
}

/// Hands the webview to a server's client; `dms` / `dm_user` ask the bridge to open Sharkord's DM
/// list or one conversation once connected.
#[tauri::command]
pub async fn select_server(
    app: AppHandle,
    store: State<'_, Store>,
    id: String,
    dms: Option<bool>,
    dm_user: Option<String>,
) -> Result<()> {
    let entry = entry_of(&store, &id)?;
    let token = app.state::<Inbox>().token(&id);

    app.state::<Showing>().set_pending_dms(dms.unwrap_or(false));
    app.state::<Showing>().set_pending_dm_user(dm_user);

    webview::show_server(&app, &entry, token.as_deref())?;
    inbox::sync(&app);

    store.update(|registry| {
        registry.settings.last_server_id = Some(id);

        Ok(())
    })
}

#[tauri::command]
pub async fn show_shiver(app: AppHandle) -> Result<()> {
    webview::show_shiver(&app)?;
    inbox::sync(&app);

    Ok(())
}

#[tauri::command]
pub fn showing_server(showing: State<'_, Showing>) -> Option<String> {
    showing.server()
}

#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
pub fn get_settings(store: State<'_, Store>) -> Settings {
    store.registry().settings.clone().sanitised()
}

/// Saves settings (sanitised; bookkeeping fields kept) and repaints the gap between pages.
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    store: State<'_, Store>,
    settings: Settings,
) -> Result<()> {
    let saved = store.update(|registry| {
        registry.settings = Settings {
            last_server_id: registry.settings.last_server_id.take(),
            skipped_update: registry.settings.skipped_update.take(),
            push_servers: std::mem::take(&mut registry.settings.push_servers),
            ..settings.sanitised()
        };

        Ok(registry.settings.clone())
    })?;

    if let Ok(window) = webview::main_window(&app) {
        let _ = window.set_background_color(Some(webview::background_color(&saved)));
    }

    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RailRef {
    pub kind: String,
    pub id: String,
}

/// Applies a drag at the top level (servers and folders share one position space).
#[tauri::command]
pub async fn reorder_rail(store: State<'_, Store>, ordered: Vec<RailRef>) -> Result<()> {
    store.update(|registry| {
        for (index, item) in ordered.iter().enumerate() {
            let position = index as i32;

            match item.kind.as_str() {
                "folder" => {
                    registry
                        .folder_mut(&item.id)
                        .ok_or(Error::UnknownFolder)?
                        .position = position
                }
                "server" => {
                    registry
                        .server_mut(&item.id)
                        .ok_or(Error::UnknownServer)?
                        .position = position
                }
                other => {
                    return Err(Error::InvalidInput(format!(
                        "'{other}' is not a kind of rail item"
                    )))
                }
            }
        }

        Ok(())
    })
}

#[tauri::command]
pub async fn reorder_servers(store: State<'_, Store>, ordered_ids: Vec<String>) -> Result<()> {
    store.update(|registry| {
        for (index, id) in ordered_ids.iter().enumerate() {
            registry
                .server_mut(id)
                .ok_or(Error::UnknownServer)?
                .position = index as i32;
        }

        registry.servers.sort_by_key(|server| server.position);

        Ok(())
    })
}

/// Creates a folder holding `member_ids`, at the first member's place, in one step.
#[tauri::command]
pub async fn create_folder_with(
    store: State<'_, Store>,
    name: String,
    member_ids: Vec<String>,
) -> Result<Folder> {
    store.update(|registry| {
        let position = registry
            .servers
            .iter()
            .filter(|server| member_ids.contains(&server.id))
            .map(|server| server.position)
            .min()
            .unwrap_or_else(|| registry.next_position());

        let folder = Folder {
            id: Uuid::new_v4().to_string(),
            name: clamp_name(&name)?,
            position,
            expanded: true,
        };

        for (index, id) in member_ids.iter().enumerate() {
            let server = registry.server_mut(id).ok_or(Error::UnknownServer)?;

            server.folder_id = Some(folder.id.clone());
            server.position = index as i32;
        }

        registry.folders.push(folder.clone());

        Ok(folder)
    })
}

/// Moves a server into a folder (or out, with `None`); a folder left with one server dissolves.
#[tauri::command]
pub async fn set_server_folder(
    store: State<'_, Store>,
    id: String,
    folder_id: Option<String>,
) -> Result<()> {
    store.update(|registry| {
        if let Some(folder_id) = &folder_id {
            registry.folder_mut(folder_id).ok_or(Error::UnknownFolder)?;
        }

        registry
            .server_mut(&id)
            .ok_or(Error::UnknownServer)?
            .folder_id = folder_id;
        crate::model::prune_folders(registry);

        Ok(())
    })
}

#[tauri::command]
pub async fn rename_folder(store: State<'_, Store>, id: String, name: String) -> Result<()> {
    store.update(|registry| {
        registry.folder_mut(&id).ok_or(Error::UnknownFolder)?.name = clamp_name(&name)?;

        Ok(())
    })
}

/// Deletes a folder; its servers move back to the top level.
#[tauri::command]
pub async fn delete_folder(store: State<'_, Store>, id: String) -> Result<()> {
    store.update(|registry| {
        registry.folder_mut(&id).ok_or(Error::UnknownFolder)?;

        for server in registry
            .servers
            .iter_mut()
            .filter(|server| server.folder_id.as_deref() == Some(id.as_str()))
        {
            server.folder_id = None;
        }

        registry.folders.retain(|folder| folder.id != id);

        Ok(())
    })
}

#[tauri::command]
pub async fn set_folder_expanded(
    store: State<'_, Store>,
    id: String,
    expanded: bool,
) -> Result<()> {
    store.update(|registry| {
        registry
            .folder_mut(&id)
            .ok_or(Error::UnknownFolder)?
            .expanded = expanded;

        Ok(())
    })
}
