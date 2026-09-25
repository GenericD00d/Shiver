//! Commands called from Shiver's own webviews (the shell, bell and popup). Server pages have no IPC.
//!
//! Every command that writes the registry is `async`, so its disk write happens on a worker rather
//! than the UI thread. Passwords arrive as `Zeroizing<String>` so Shiver's copies are wiped.

use tauri::{
    menu::{ContextMenu, MenuBuilder, MenuItemBuilder},
    AppHandle, Emitter, Manager, State,
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    drain::{self, Readiness},
    error::{Core, Error, Result},
    feed::{DmEntry, Feed, Notification},
    hotkey, jwt,
    model::{normalize_origin, Folder, Registry, ServerEntry, Settings},
    secrets::{self, Secret},
    session::Recovery,
    store::{RegistryStore, Store},
    voice::{VoiceState, VoiceStatus},
    webviews,
};

use sharkord_client::{CheckedSessions, ServerCheck};
use shiver_core::{login, probe};

/// Tells Shiver's own webviews the settings changed.
const SETTINGS_EVENT: &str = "shiver://settings";

type Password = Zeroizing<String>;

/* ── lookups ── */

fn entry_of(store: &Store, id: &str) -> Result<ServerEntry> {
    store
        .registry()
        .server(id)
        .cloned()
        .ok_or(Core::UnknownServer.into())
}

/// The entry, the settings and its mutes: what building a page needs.
fn page_inputs(store: &Store, id: &str) -> Result<(ServerEntry, Settings, Vec<i64>)> {
    let registry = store.registry();
    let entry = registry.server(id).cloned().ok_or(Core::UnknownServer)?;

    Ok((entry, registry.settings.clone(), registry.muted_for(id)))
}

fn voice_locked_for(app: &AppHandle, entry_id: &str) -> bool {
    app.state::<VoiceState>()
        .holder()
        .is_some_and(|holder| holder != entry_id)
}

/* ── sessions ── */

/// A session token for this entry, signing in again when the stored one is due for renewal.
///
/// Never fails: with no usable session the server's own login page is the fallback. A token that
/// is due for renewal but still valid is used when renewal is impossible (no password) or fails.
pub(crate) async fn ensure_session(entry: &ServerEntry) -> Option<String> {
    let stored = secrets::read_off_thread(Secret::Session, &entry.id).await;

    if let Some(token) = stored.as_deref() {
        if !jwt::needs_refresh(token) {
            return Some(token.to_string());
        }
    }

    let still_live = stored
        .as_deref()
        .filter(|token| jwt::is_live(token))
        .map(|token| token.to_string());

    let (Some(identity), Some(password)) = (
        entry.identity.as_deref(),
        secrets::read_off_thread(Secret::Password, &entry.id).await,
    ) else {
        return still_live;
    };

    match login::sign_in(&entry.origin, identity, &password).await {
        Ok(token) => {
            let _ = secrets::store_off_thread(Secret::Session, &entry.id, &token).await;

            Some(token)
        }
        Err(error) => {
            eprintln!(
                "[shiver] could not refresh the session for {}: {error}",
                entry.origin
            );

            still_live
        }
    }
}

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

/* ── adding, checking and removing servers ── */

#[tauri::command]
pub fn list_registry(store: State<'_, Store>) -> Registry {
    Registry::clone(&store.registry())
}

/// See [`sharkord_client::check_server`].
#[tauri::command]
pub async fn check_server(
    kept: State<'_, CheckedSessions>,
    origin: String,
    identity: Option<String>,
    password: Option<Password>,
) -> Result<ServerCheck> {
    Ok(sharkord_client::check_server(
        &origin,
        identity.as_deref(),
        password.as_deref().map(String::as_str),
        &kept,
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
        let label = account_label
            .clone()
            .or_else(|| identity.clone())
            .or_else(|| (existing > 0).then(|| format!("Account {}", existing + 1)));

        let entry = ServerEntry {
            id: id.clone(),
            origin: origin.clone(),
            server_id: Some(info.server_id.clone()),
            name: info.name.clone(),
            icon_url: info.icon_url.clone(),
            identity,
            account_label: label,
            folder_id: None,
            position: registry.next_position(),
            accept_any_size: false,
            profile: None,
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
    webviews::close_dm_view(&app, &id);

    // credentials go first, so a failure cannot leave one behind for a server no longer listed
    secrets::forget_all_off_thread(&id).await?;

    feed.forget_entry(&id);
    app.state::<VoiceState>().forget_entry(&id);
    app.state::<Readiness>().forget_entry(&id);
    app.state::<Recovery>().forget_entry(&id);
    app.state::<webviews::Openings>().forget(&id);

    store.update(|registry| {
        registry.servers.retain(|server| server.id != id);
        registry.muted.retain(|muted| muted.entry_id != id);
        registry.baselines.remove(&id);
        registry.rail().prune_folders();
        registry
            .pending_permission_resets
            .retain(|pending| pending != &id);

        if registry.settings.last_server_id.as_deref() == Some(id.as_str()) {
            registry.settings.last_server_id = None;
        }

        Ok(())
    })?;

    webviews::discard_profiles(&app, &id, None);
    crate::watch::sync(&app);
    drain::notify_feed_changed(&app);

    Ok(())
}

/// Signs a server out and leaves it signed out: session, password and username are dropped, and its
/// browser profile is replaced so the page keeps no cookie or stored token either.
#[tauri::command]
pub async fn log_out_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    secrets::forget_all_off_thread(&id).await?;

    webviews::close_server(&app, &id)?;
    webviews::close_dm_view(&app, &id);

    let entry = store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Core::UnknownServer)?;

        server.identity = None;
        server.profile = Some(Uuid::new_v4().simple().to_string());

        Ok(server.clone())
    })?;

    webviews::discard_profiles(&app, &id, Some(&entry));
    app.state::<Recovery>().forget_entry(&id);
    app.state::<Readiness>().forget_entry(&id);

    let settings = store.registry().settings.clone();
    let locked = voice_locked_for(&app, &id);

    webviews::show_server(&app, &entry, &settings, None, &[], locked)?;
    webviews::preload_dm_view(&app, &entry, &settings, None)?;
    crate::watch::sync(&app);

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
    webviews::close_dm_view(&app, &id);
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

/// Forgets stored camera/microphone answers. Answers how many were forgotten now.
///
/// On a blocking thread: WebView2 answers on the main thread, so waiting there would deadlock.
#[tauri::command]
pub async fn reset_media_permissions(app: AppHandle) -> Result<usize> {
    tauri::async_runtime::spawn_blocking(move || crate::permissions::clear_media_permissions(&app))
        .await
        .map_err(|error| Error::Webview(error.to_string()))?
}

#[tauri::command]
pub async fn refresh_server_info(store: State<'_, Store>, id: String) -> Result<ServerEntry> {
    let info = probe::fetch_info(&entry_of(&store, &id)?.origin).await?;

    store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Core::UnknownServer)?;

        server.name = info.name.clone();
        server.icon_url = info.icon_url.clone();
        server.server_id = Some(info.server_id.clone());

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

/* ── the rail ── */

/// Applies a drag of top-level servers only.
#[tauri::command]
pub async fn reorder_servers(store: State<'_, Store>, ordered_ids: Vec<String>) -> Result<()> {
    store.update(|registry| Ok(registry.rail().reorder_servers(&ordered_ids)?))
}

/// Applies a drag anywhere at the top level (servers and folders share one position space).
#[tauri::command]
pub async fn reorder_rail(
    store: State<'_, Store>,
    ordered: Vec<shiver_core::rail::RailRef>,
) -> Result<()> {
    store.update(|registry| Ok(registry.rail().reorder(&ordered)?))
}

/// Creates a folder holding `member_ids`, at the first member's place, in one step.
#[tauri::command]
pub async fn create_folder_with(
    store: State<'_, Store>,
    name: String,
    member_ids: Vec<String>,
) -> Result<Folder> {
    store.update(|registry| {
        let mut rail = registry.rail();
        let folder = rail.create_folder(Uuid::new_v4().to_string(), &name, &member_ids)?;

        rail.prune_folders();

        Ok(folder)
    })
}

#[tauri::command]
pub async fn rename_folder(store: State<'_, Store>, id: String, name: String) -> Result<()> {
    store.update(|registry| Ok(registry.rail().rename_folder(&id, &name)?))
}

/// Deletes a folder; its servers move back to the top level.
#[tauri::command]
pub async fn delete_folder(store: State<'_, Store>, id: String) -> Result<()> {
    store.update(|registry| Ok(registry.rail().delete_folder(&id)?))
}

/// Moves a server into a folder (or out, with `None`); a folder left with one server dissolves.
#[tauri::command]
pub async fn set_server_folder(
    store: State<'_, Store>,
    id: String,
    folder_id: Option<String>,
) -> Result<()> {
    store.update(|registry| {
        let mut rail = registry.rail();

        rail.set_server_folder(&id, folder_id)?;
        rail.prune_folders();

        Ok(())
    })
}

#[tauri::command]
pub async fn set_folder_expanded(
    store: State<'_, Store>,
    id: String,
    expanded: bool,
) -> Result<()> {
    store.update(|registry| Ok(registry.rail().set_folder_expanded(&id, expanded)?))
}

#[tauri::command]
pub async fn show_folder_menu(app: AppHandle, id: String) -> Result<()> {
    let window = webviews::main_window(&app)?;
    let rename =
        MenuItemBuilder::with_id(format!("rename-folder:{id}"), "Rename folder").build(&app)?;
    let delete =
        MenuItemBuilder::with_id(format!("delete-folder:{id}"), "Delete folder").build(&app)?;

    MenuBuilder::new(&app)
        .item(&rename)
        .separator()
        .item(&delete)
        .build()?
        .popup(window)?;

    Ok(())
}

/// The rail's context menu, as a native menu (an html one would be painted over by the server
/// webview). The target is encoded in each item's id; `lib.rs` forwards the choice to the shell.
#[tauri::command]
pub async fn show_server_menu(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    let window = webviews::main_window(&app)?;
    let entry = entry_of(&store, &id)?;
    let item = |action: &str, label: &str| {
        MenuItemBuilder::with_id(format!("{action}:{id}"), label).build(&app)
    };

    let open = item("open", "Open")?;
    let mark_read = item("markread", "Mark all as read")?;
    let refresh = item("refresh", "Refresh name and icon")?;
    let remove = item("remove", "Remove from Shiver")?;
    let session = if entry.identity.is_some() {
        item("logout", "Log out")?
    } else {
        item("signin", "Sign in")?
    };

    // enabled only when there is a password to forget (a keychain read, off the UI thread)
    let has_password = secrets::read_off_thread(Secret::Password, &id)
        .await
        .is_some();
    let forget = MenuItemBuilder::with_id(format!("forgetpw:{id}"), "Forget my password")
        .enabled(has_password)
        .build(&app)?;

    let plugin = MenuItemBuilder::with_id(
        format!("plugin:{id}"),
        plugin_menu_label(app.state::<crate::watch::Plugins>().all().get(&id)),
    )
    .enabled(false)
    .build(&app)?;

    let mut builder =
        MenuBuilder::new(&app).items(&[&open, &mark_read, &refresh, &forget, &plugin]);

    // the size limit is only offered where it matters: a server that tripped it, or one raised
    let sizes = if entry.accept_any_size {
        Some(item("normalsize", "Back to the normal size limit")?)
    } else if app.state::<crate::watch::Reported>().mentioned(&id) {
        Some(item("anysize", "Accept larger messages from this server")?)
    } else {
        None
    };

    if let Some(sizes) = &sizes {
        builder = builder.item(sizes);
    }

    let take_out = entry
        .folder_id
        .is_some()
        .then(|| item("unfolder", "Move out of folder"))
        .transpose()?;

    if let Some(take_out) = &take_out {
        builder = builder.separator().item(take_out);
    }

    builder
        .separator()
        .item(&session)
        .item(&remove)
        .build()?
        .popup(window)?;

    Ok(())
}

/// Absent = not connected yet, `None` = not installed.
fn plugin_menu_label(status: Option<&Option<String>>) -> String {
    match status {
        Some(Some(version)) => format!("\u{2713} Shiver plugin {version}"),
        Some(None) => "\u{2717} No Shiver plugin".to_string(),
        None => "Shiver plugin: not checked yet".to_string(),
    }
}

/* ── showing servers ── */

/// Shows a server, creating its page on first use, and remembers it for next launch.
///
/// Async: creating a webview blocks on the event loop, which a sync command (on the main thread)
/// would deadlock.
#[tauri::command]
pub async fn select_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    let (entry, settings, muted) = page_inputs(&store, &id)?;
    let token = ensure_session(&entry).await;

    webviews::show_server(
        &app,
        &entry,
        &settings,
        token.as_deref(),
        &muted,
        voice_locked_for(&app, &id),
    )?;
    webviews::trim_pages(&app);
    crate::watch::publish_floor(&app, &id);

    store.update(|registry| {
        registry.settings.last_server_id = Some(id);

        Ok(())
    })
}

/// Brings a server's page up hidden and says whether its client is already connected, so the shell
/// can cover it until `shiver://server-ready`.
#[tauri::command]
pub async fn prepare_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<bool> {
    if app.get_webview(&webviews::webview_label(&id)).is_none() {
        let (entry, settings, muted) = page_inputs(&store, &id)?;
        let token = ensure_session(&entry).await;

        webviews::preload_server(
            &app,
            &entry,
            &settings,
            token.as_deref(),
            &muted,
            voice_locked_for(&app, &id),
        )?;

        // the page reports for itself now, so its socket stands down
        crate::watch::sync(&app);
    }

    Ok(app.state::<Readiness>().is_ready(&id))
}

#[tauri::command]
pub async fn show_shell(app: AppHandle) -> Result<()> {
    webviews::show_shell_only(&app)
}

#[tauri::command]
pub async fn exit_dm_split(app: AppHandle) -> Result<()> {
    webviews::hide_dm_views(&app)
}

/// Opens one conversation in the entry's conversation view, beside Shiver's DM list.
#[tauri::command]
pub async fn open_dm(
    app: AppHandle,
    store: State<'_, Store>,
    entry_id: String,
    name: String,
) -> Result<()> {
    let (entry, settings, _) = page_inputs(&store, &entry_id)?;
    let token = ensure_session(&entry).await;

    let created = webviews::show_dm_view(&app, &entry, &settings, token.as_deref(), &name)?;

    webviews::trim_pages(&app);

    if created {
        return Ok(());
    }

    if let Some(webview) = app.get_webview(&webviews::dm_webview_label(&entry_id)) {
        let payload =
            serde_json::to_string(&name).map_err(|error| Error::Webview(error.to_string()))?;

        webview.eval(format!(
            "window.__SHIVER_OPEN_DM__ && window.__SHIVER_OPEN_DM__({payload})"
        ))?;
    }

    Ok(())
}

/// From the popup: asks the shell to go to a message, and closes the popup.
#[tauri::command]
pub async fn open_message(
    app: AppHandle,
    entry_id: String,
    channel_id: Option<i64>,
    is_dm: bool,
    author: String,
) -> Result<()> {
    let _ = app.emit_to(
        webviews::SHELL_WEBVIEW,
        drain::OPEN_MESSAGE_EVENT,
        serde_json::json!({ "entryId": entry_id, "channelId": channel_id, "isDm": is_dm, "author": author }),
    );

    webviews::set_popup_open(&app, false)
}

/// Puts a server's page on one channel (the shell has already opened the server).
#[tauri::command]
pub fn select_channel(app: AppHandle, entry_id: String, channel_id: i64) -> Result<()> {
    if let Some(webview) = app.get_webview(&webviews::webview_label(&entry_id)) {
        webview.eval(format!(
            "window.__SHIVER_SELECT_CHANNEL__ && window.__SHIVER_SELECT_CHANNEL__({channel_id})"
        ))?;
    }

    Ok(())
}

/* ── voice ── */

#[tauri::command]
pub fn voice_status(voice: State<'_, VoiceState>) -> Option<VoiceStatus> {
    voice.status()
}

/// Works the call's controls on whichever entry holds it.
#[tauri::command]
pub async fn voice_control(app: AppHandle, action: String) -> Result<()> {
    if !matches!(action.as_str(), "mic" | "sound" | "leave") {
        return Err(Core::InvalidInput(format!("Unknown voice action '{action}'")).into());
    }

    let voice = app.state::<VoiceState>();

    if let Some(holder) = voice.holder() {
        webviews::run_voice_action(&app, &holder, &action);

        // a page that does not really leave loses the call anyway; only the page on screen can
        // start one again
        if action == "leave" {
            voice.forget_entry(&holder);
            let _ = app.emit_to(webviews::SHELL_WEBVIEW, drain::VOICE_EVENT, ());
        }
    }

    Ok(())
}

/* ── settings ── */

#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
pub fn get_settings(store: State<'_, Store>) -> Settings {
    store.registry().settings.clone().sanitised()
}

/// Saves settings (sanitised; Shiver's own bookkeeping fields are kept) and applies them live.
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
            ..settings.sanitised()
        };

        Ok(registry.settings.clone())
    })?;

    hotkey::apply(&app, saved.mute_hotkey.as_deref());
    webviews::trim_pages(&app);
    webviews::push_theme(&app, &saved);

    for label in [
        webviews::SHELL_WEBVIEW,
        webviews::OVERLAY_WEBVIEW,
        webviews::POPUP_WEBVIEW,
    ] {
        let _ = app.emit_to(label, SETTINGS_EVENT, ());
    }

    Ok(())
}

/* ── notifications and mutes ── */

#[tauri::command]
pub fn list_notifications(feed: State<'_, Feed>) -> Vec<Notification> {
    feed.notifications()
}

#[tauri::command]
pub fn list_dms(feed: State<'_, Feed>) -> Vec<DmEntry> {
    feed.dms()
}

/// The bell's count: the feed only, since every entry there has a message to show.
#[tauri::command]
pub fn unread_count(feed: State<'_, Feed>) -> usize {
    feed.unread_count()
}

/// Per-entry rail badges: the feed's unread plus what arrived while Shiver was closed.
#[tauri::command]
pub fn unread_counts(
    app: AppHandle,
    feed: State<'_, Feed>,
) -> std::collections::HashMap<String, usize> {
    let mut counts = feed.unread_by_entry();

    for (entry_id, missed) in app.state::<crate::watch::Missed>().counts() {
        *counts.entry(entry_id).or_default() += missed;
    }

    counts
}

/// "Mark all as read": Shiver's notifications, the missed count, and Sharkord's own channel state.
#[tauri::command]
pub async fn mark_server_read(app: AppHandle, entry_id: String) -> Result<()> {
    webviews::mark_all_read(&app, &entry_id);
    crate::watch::mark_read(&app, &entry_id);

    if app.state::<Feed>().mark_entry_read(&entry_id) {
        drain::notify_feed_changed(&app);
    }

    Ok(())
}

#[tauri::command]
pub fn mark_notifications_read(app: AppHandle, feed: State<'_, Feed>) {
    feed.mark_all_read();
    drain::notify_feed_changed(&app);
}

#[tauri::command]
pub fn clear_notifications(app: AppHandle, feed: State<'_, Feed>) {
    feed.clear();
    drain::notify_feed_changed(&app);
}

#[tauri::command]
pub async fn set_channel_muted(
    app: AppHandle,
    store: State<'_, Store>,
    entry_id: String,
    channel_id: i64,
    muted: bool,
) -> Result<()> {
    let remaining = store.update(|registry| {
        let mut channels = registry.muted_for(&entry_id);

        channels.retain(|channel| *channel != channel_id);

        if muted {
            channels.push(channel_id);
        }

        Ok(registry.set_muted_for(&entry_id, channels))
    })?;

    webviews::push_muted(&app, &entry_id, &remaining);

    Ok(())
}

/* ── popup ── */

/// Toggles the popup; returns the new state.
#[tauri::command]
pub async fn toggle_popup(app: AppHandle) -> Result<bool> {
    let open = app.state::<webviews::ActiveServer>().should_open_popup();

    webviews::set_popup_open(&app, open)?;

    Ok(open)
}

#[tauri::command]
pub async fn close_popup(app: AppHandle) -> Result<()> {
    webviews::set_popup_open(&app, false)
}

/// The popup lost focus; arms the reopen guard so the click that caused it does not reopen it.
#[tauri::command]
pub async fn dismiss_popup(app: AppHandle) -> Result<()> {
    app.state::<webviews::ActiveServer>().note_popup_dismissed();
    webviews::set_popup_open(&app, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_label_has_three_answers() {
        assert!(plugin_menu_label(Some(&Some("0.2.0".into()))).contains("0.2.0"));
        assert!(plugin_menu_label(Some(&None)).contains("No"));
        assert!(plugin_menu_label(None).contains("not checked"));
    }
}
