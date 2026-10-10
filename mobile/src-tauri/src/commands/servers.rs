//! Adding, checking, signing in to, logging out of and removing servers, their sessions and their
//! per-server switches.

use std::collections::HashMap;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_shiver_secrets::SecretsExt;
use uuid::Uuid;
use zeroize::Zeroizing;

use sharkord_client::{CheckedSessions, ServerCheck};
use shiver_core::{limit::Joins, login, model::NotifyLevel, probe};

use crate::{
    error::{Core, Result},
    icons,
    inbox::{self, Inbox},
    model::{normalize_origin, Registry, ServerEntry, ServerInfo},
    store::{RegistryStore, Store},
};

use super::{entry_of, Password};

/// Clears what an origin's pages stored. The one webview has one storage per origin, so this also
/// signs out the pages of any other account on the same address; leaving it would keep this one's.
fn wipe_page(app: &AppHandle, origin: &str) {
    if let Err(error) = app.shiver_secrets().wipe_origin(origin) {
        eprintln!("[shiver] could not clear {origin} from the webview: {error}");
    }
}

#[tauri::command]
pub fn list_registry(store: State<'_, Store>) -> Registry {
    Registry::clone(&store.registry())
}

#[tauri::command]
pub async fn probe_server(origin: String) -> Result<ServerInfo> {
    Ok(probe::fetch_info(&normalize_origin(&origin)?).await?)
}

/// See [`sharkord_client::check_server`].
#[tauri::command]
pub async fn check_server(
    kept: State<'_, CheckedSessions>,
    joins: State<'_, Joins>,
    origin: String,
    identity: Option<String>,
    password: Option<Password>,
) -> Result<ServerCheck> {
    Ok(sharkord_client::check_server(
        &origin,
        identity.as_deref(),
        password.as_deref().map(String::as_str),
        &kept,
        &joins,
    )
    .await?)
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
    account_label: Option<String>,
    remember_password: Option<bool>,
) -> Result<ServerEntry> {
    let origin = normalize_origin(&origin)?;
    let info = probe::fetch_info(&origin).await?;
    let identity = identity
        .map(|identity| identity.trim().to_string())
        .filter(|identity| !identity.is_empty());
    let password = password.filter(|password| !password.is_empty());

    let session = match (&identity, &password) {
        (Some(identity), Some(password)) => Some(
            match app
                .state::<CheckedSessions>()
                .take(&origin, identity, password)
            {
                Some(token) => token,
                None => Zeroizing::new(login::sign_in(&origin, identity, password).await?),
            },
        ),
        _ => None,
    };

    let icon = match info.icon_url.as_deref() {
        Some(url) => probe::fetch_icon(url).await,
        None => None,
    };

    let entry = store.update(|registry| {
        let existing = registry
            .servers
            .iter()
            .filter(|server| server.origin == origin)
            .count();
        let identity = identity.clone().filter(|_| session.is_some());
        // a second account on one origin needs a label to be told apart
        let account_label = shiver_core::model::account_label(account_label.as_deref())
            .or_else(|| identity.clone())
            .or_else(|| (existing > 0).then(|| format!("Account {}", existing + 1)));

        let entry = ServerEntry {
            id: Uuid::new_v4().to_string(),
            origin: origin.clone(),
            name: info.name.clone(),
            icon_url: info.icon_url.clone(),
            identity,
            account_label,
            position: registry.next_position(),
            push_token: Some(Uuid::new_v4().simple().to_string()),
            ..Default::default()
        };

        registry.servers.push(entry.clone());

        Ok(entry)
    })?;

    icons::save(&app, &entry.id, icon.as_deref());

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
    let origin = entry_of(&store, &id)?.origin;

    // before the entry goes: unregistering needs its push token
    crate::push::unregister(&app, &id);
    inbox::forget_everywhere(&app, &id);
    wipe_page(&app, &origin);
    icons::save(&app, &id, None);

    store.update(|registry| {
        registry.servers.retain(|server| server.id != id);
        registry.muted.retain(|muted| muted.entry_id != id);
        shiver_core::links::forget_entry(&mut registry.settings.trusted_links, &id);
        registry
            .settings
            .push_servers
            .retain(|pushed| pushed != &id);
        registry.rail().prune_folders();

        if registry.settings.last_server_id.as_deref() == Some(id.as_str()) {
            registry.settings.last_server_id = None;
        }

        Ok(())
    })
}

/// Re-reads a server's name and logo.
#[tauri::command]
pub async fn refresh_server_info(
    app: AppHandle,
    store: State<'_, Store>,
    id: String,
) -> Result<ServerEntry> {
    let info = probe::fetch_info(&entry_of(&store, &id)?.origin).await?;
    let icon = match info.icon_url.as_deref() {
        Some(url) => probe::fetch_icon(url).await,
        None => None,
    };

    let entry = store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Core::UnknownServer)?;

        server.name = info.name.clone();
        server.icon_url = info.icon_url.clone();

        Ok(server.clone())
    })?;

    icons::save(&app, &id, icon.as_deref());

    Ok(entry)
}

/// Every server's logo as a `data:` uri, by entry id.
#[tauri::command]
pub fn server_icons(app: AppHandle, store: State<'_, Store>) -> HashMap<String, String> {
    let ids: Vec<String> = store
        .registry()
        .servers
        .iter()
        .map(|server| server.id.clone())
        .collect();

    icons::load(&app, &ids)
}

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
        inbox::forget_credentials(&app, &id);
    }
}

/// Logs out of a server: stops its push wake-ups, forgets Shiver's session, password and identity
/// for it, and wipes what its page stored in the webview.
#[tauri::command]
pub async fn log_out_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    if let Err(error) = crate::push::set_wanted(&app, &id, false) {
        eprintln!("[shiver] could not stop push for {id}: {error}");
    }

    let origin = store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Core::UnknownServer)?;

        server.identity = None;

        Ok(server.origin.clone())
    })?;

    inbox::forget_everywhere(&app, &id);
    wipe_page(&app, &origin);

    Ok(())
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
            .ok_or(Core::UnknownServer)?
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

/// Raises (or restores) the message size limit, and reconnects with it.
#[tauri::command]
pub async fn set_accept_any_size(
    app: AppHandle,
    store: State<'_, Store>,
    id: String,
    accept: bool,
) -> Result<()> {
    store.update(|registry| {
        registry
            .server_mut(&id)
            .ok_or(Core::UnknownServer)?
            .accept_any_size = accept;

        Ok(())
    })?;

    inbox::restart(&app, &id);

    Ok(())
}

/// Sets which of a server's messages notify (only the core's own connections notify on Android).
#[tauri::command]
pub async fn set_notify_level(
    store: State<'_, Store>,
    id: String,
    level: NotifyLevel,
) -> Result<()> {
    store.update(|registry| {
        registry.server_mut(&id).ok_or(Core::UnknownServer)?.notify = level;

        Ok(())
    })
}

/// What Shiver's screens show about each server's session: waiting for a sign-in, a password kept,
/// a problem watching it that will not fix itself, and the companion plugin's version (`None`:
/// connected, not installed; absent: not connected yet).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStates {
    pub signed_out: Vec<String>,
    pub remembered: Vec<String>,
    pub problems: HashMap<String, String>,
    pub plugins: HashMap<String, Option<String>>,
}

#[tauri::command]
pub fn session_states(inbox: State<'_, Inbox>, store: State<'_, Store>) -> SessionStates {
    SessionStates {
        signed_out: inbox.signed_out(),
        remembered: store
            .registry()
            .servers
            .iter()
            .filter(|server| inbox.has_password(&server.id))
            .map(|server| server.id.clone())
            .collect(),
        problems: inbox.problems(),
        plugins: inbox.plugins(),
    }
}
