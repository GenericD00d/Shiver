//! Adding, checking, signing in to, logging out of and removing servers, and their per-server switches.

use sharkord_client::{CheckedSessions, ServerCheck};
use shiver_core::{limit::Joins, login, model::NotifyLevel, probe};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    drain::{self, Readiness},
    error::{Core, Result},
    feed::Feed,
    model::{normalize_origin, RegistryView, ServerEntry},
    secrets::{self, Secret},
    session::{self, Recovery},
    store::{RegistryStore, Store},
    voice::VoiceState,
    webviews,
};

use super::{entry_of, voice_locked_for, Password};

/// Stores a fresh session (and the password, or forgets it). On failure nothing is left behind.
async fn store_credentials(
    id: &str,
    token: &str,
    password: &str,
    remember_password: bool,
) -> Result<()> {
    let stored = async {
        secrets::store_off_thread(Secret::Session, id, token).await?;

        if remember_password {
            secrets::store_off_thread(Secret::Password, id, password).await
        } else {
            secrets::forget_off_thread(Secret::Password, id).await
        }
    }
    .await;

    if stored.is_err() {
        let _ = secrets::forget_all_off_thread(id).await;
    }

    stored
}

#[tauri::command]
pub fn list_registry(store: State<'_, Store>) -> RegistryView {
    store.registry().view()
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

/// Adds a server, signing in first when credentials are given (so a typo adds nothing). The password
/// is kept unless `remember_password` is `false`; it is what lets Shiver renew the week-long session.
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
    let id = Uuid::new_v4().to_string();

    // credentials first: an entry is only written once its secrets are safely stored
    let signed_in = match (
        &identity,
        password.as_ref().filter(|password| !password.is_empty()),
    ) {
        (Some(identity), Some(password)) => {
            let token = match app
                .state::<CheckedSessions>()
                .take(&origin, identity, password)
            {
                Some(token) => token,
                None => Zeroizing::new(login::sign_in(&origin, identity, password).await?),
            };

            store_credentials(&id, &token, password, remember_password != Some(false)).await?;

            true
        }
        _ => false,
    };

    let added = store.update(|registry| {
        let existing = registry
            .servers
            .iter()
            .filter(|server| server.origin == origin)
            .count();
        let identity = identity.clone().filter(|_| signed_in);

        // a second account on one origin needs a label to be told apart
        let label = shiver_core::model::account_label(account_label.as_deref())
            .or_else(|| identity.clone())
            .or_else(|| (existing > 0).then(|| format!("Account {}", existing + 1)));

        let entry = ServerEntry {
            id: id.clone(),
            origin: origin.clone(),
            name: info.name.clone(),
            icon_url: info.icon_url.clone(),
            identity,
            account_label: label,
            folder_id: None,
            position: registry.next_position(),
            accept_any_size: false,
            profile: None,
            media_allowed: false,
            notify: NotifyLevel::default(),
        };

        registry.servers.push(entry.clone());

        Ok(entry)
    });

    if added.is_err() && signed_in {
        let _ = secrets::forget_all_off_thread(&id).await;
    }

    crate::watch::sync(&app);

    added
}

/// Removes a server and everything Shiver keeps about it: pages, browser profile, credentials,
/// notifications, mutes and unread floor.
#[tauri::command]
pub async fn remove_server(
    app: AppHandle,
    store: State<'_, Store>,
    feed: State<'_, Feed>,
    id: String,
) -> Result<()> {
    entry_of(&store, &id)?;

    webviews::close_server(&app, &id)?;

    // held to the end: a renewal finishing meanwhile must not store a session for it again
    let _held = session::CREDENTIALS.lock().await;

    // credentials go first, so a failure cannot leave one behind for a server no longer listed
    secrets::forget_all_off_thread(&id).await?;

    feed.forget_entry(&id);
    app.state::<VoiceState>().forget_entry(&id);
    app.state::<Readiness>().forget_entry(&id);
    app.state::<Recovery>().forget_entry(&id);
    app.state::<webviews::Openings>().forget(&id);
    app.state::<crate::notify::Posted>().forget_entry(&id);

    store.update(|registry| {
        registry.servers.retain(|server| server.id != id);
        registry.muted.retain(|muted| muted.entry_id != id);
        shiver_core::links::forget_entry(&mut registry.settings.trusted_links, &id);
        registry.baselines.remove(&id);
        registry.rail().prune_folders();

        if registry.settings.last_server_id.as_deref() == Some(id.as_str()) {
            registry.settings.last_server_id = None;
        }

        Ok(())
    })?;

    webviews::discard_profiles(&app, &id, None);
    crate::watch::sync(&app);
    crate::watch::forget(&app, &id);
    drain::notify_feed_changed(&app);

    Ok(())
}

/// Signs a server out and leaves it signed out: session, password and username are dropped, and its
/// browser profile is replaced so the page keeps no cookie or stored token either.
#[tauri::command]
pub async fn log_out_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    // a renewal finishing meanwhile must not store a session again: it waits, then sees no identity
    let held = session::CREDENTIALS.lock().await;

    secrets::forget_all_off_thread(&id).await?;

    let on_screen = app.state::<webviews::ActiveServer>().is_on_screen(&id);

    webviews::close_server(&app, &id)?;

    let entry = store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Core::UnknownServer)?;

        server.identity = None;
        server.profile = Some(Uuid::new_v4().simple().to_string());
        server.media_allowed = false;

        Ok(server.clone())
    })?;

    drop(held);

    webviews::discard_profiles(&app, &id, Some(&entry));
    // the account's messages and conversations go with it (the bell and DM list are told below)
    app.state::<Feed>().forget_entry(&id);
    app.state::<crate::notify::Posted>().forget_entry(&id);
    app.state::<Recovery>().forget_entry(&id);
    app.state::<Readiness>().forget_entry(&id);

    // the page comes back at its sign-in only if it was the one on screen; any other is built when
    // the user next opens it, rather than covering whatever they are looking at
    if on_screen {
        let settings = store.registry().settings.clone();
        let locked = voice_locked_for(&app, &id);

        webviews::show_server(&app, &entry, &settings, None, &[], locked)?;
    }

    crate::watch::sync(&app);
    crate::watch::forget(&app, &id);
    drain::notify_feed_changed(&app);

    Ok(())
}

/// Signs an existing entry in (added without credentials, or its password changed). The page is
/// rebuilt by the shell afterwards so the new session takes effect.
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
    let token = Zeroizing::new(login::sign_in(&origin, &identity, &password).await?);

    store_credentials(&id, &token, &password, remember_password != Some(false)).await?;

    store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Core::UnknownServer)?;

        server.account_label.get_or_insert_with(|| identity.clone());
        server.identity = Some(identity.clone());

        Ok(())
    })?;

    webviews::close_server(&app, &id)?;
    app.state::<Readiness>().forget_entry(&id);
    app.state::<Recovery>().forget_entry(&id);
    crate::watch::restart(&app, &id);

    Ok(())
}

/// Takes back the stored password (the session stays until it expires).
#[tauri::command]
pub async fn forget_password(id: String) -> Result<()> {
    secrets::forget_off_thread(Secret::Password, &id).await
}

/// Makes every server ask again before using the camera and microphone. Answers how many had a yes.
#[tauri::command]
pub async fn reset_media_permissions(app: AppHandle) -> Result<usize> {
    crate::permissions::forget_consents(&app)
}

#[tauri::command]
pub async fn refresh_server_info(store: State<'_, Store>, id: String) -> Result<ServerEntry> {
    let info = probe::fetch_info(&entry_of(&store, &id)?.origin).await?;

    store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Core::UnknownServer)?;

        server.name = info.name.clone();
        server.icon_url = info.icon_url.clone();

        Ok(server.clone())
    })
}

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

    crate::watch::restart(&app, &id);

    Ok(())
}

/// Sets which of a server's messages notify. Its page, if loaded and out of sight, is closed so it
/// comes back with Sharkord's own notification settings to match; the page on screen, or one
/// holding the call (closing it would end the call), follows at once as best it can, and exactly
/// from its next load.
#[tauri::command]
pub async fn set_notify_level(
    app: AppHandle,
    store: State<'_, Store>,
    id: String,
    level: NotifyLevel,
) -> Result<()> {
    store.update(|registry| {
        registry.server_mut(&id).ok_or(Core::UnknownServer)?.notify = level;

        Ok(())
    })?;

    if app.get_webview(&webviews::webview_label(&id)).is_some() {
        let in_call = app.state::<VoiceState>().holder().as_deref() == Some(id.as_str());

        if in_call || app.state::<webviews::ActiveServer>().is_on_screen(&id) {
            webviews::push_notify_level(&app, &id, level);
        } else {
            webviews::close_server(&app, &id)?;
            app.state::<Readiness>().forget_entry(&id);
            crate::watch::sync(&app);
        }
    }

    Ok(())
}
