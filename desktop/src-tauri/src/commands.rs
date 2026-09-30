//! Commands called from Shiver's own webviews (the shell, bell and popup). Server pages have no IPC.
//!
//! Every command that writes the registry is `async`, so its disk write happens on a worker rather
//! than the UI thread. Passwords arrive as `Zeroizing<String>` so Shiver's copies are wiped.

use tauri::{
    menu::{
        CheckMenuItemBuilder, ContextMenu, IsMenuItem, MenuBuilder, MenuItemBuilder,
        PredefinedMenuItem, SubmenuBuilder,
    },
    AppHandle, Emitter, Manager, State, Wry,
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    drain::{self, Readiness},
    error::{Core, Result},
    feed::{DmEntry, Feed, FeedSummary, Notification},
    hotkey,
    model::{normalize_origin, Folder, RegistryView, ServerEntry, Settings},
    secrets::{self, Secret},
    session::{self, Recovery},
    store::{RegistryStore, Store},
    voice::{VoiceState, VoiceStatus},
    webviews,
};

use sharkord_client::{CheckedSessions, ServerCheck};
use shiver_core::{login, model::NotifyLevel, probe};

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
pub fn list_registry(store: State<'_, Store>) -> RegistryView {
    store.registry().view()
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
    secrets::forget_all_off_thread(&id).await?;

    webviews::close_server(&app, &id)?;

    let entry = store.update(|registry| {
        let server = registry.server_mut(&id).ok_or(Core::UnknownServer)?;

        server.identity = None;
        server.profile = Some(Uuid::new_v4().simple().to_string());
        server.media_allowed = false;

        Ok(server.clone())
    })?;

    webviews::discard_profiles(&app, &id, Some(&entry));
    app.state::<Recovery>().forget_entry(&id);
    app.state::<Readiness>().forget_entry(&id);

    let settings = store.registry().settings.clone();
    let locked = voice_locked_for(&app, &id);

    webviews::show_server(&app, &entry, &settings, None, &[], locked)?;
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

/// What a server's menu says beyond its entry (the menu itself is built by the shell, from the item
/// model both clients share).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuFacts {
    /// Shiver keeps its password, so there is one to forget
    has_password: bool,
    /// whether Shiver has connected and looked for the companion plugin yet
    plugin_checked: bool,
    /// the plugin's version, when it was found
    plugin: Option<String>,
    /// it has sent a message over the size limit this run
    too_large: bool,
}

#[tauri::command]
pub async fn server_menu_facts(app: AppHandle, id: String) -> MenuFacts {
    // a keychain read, off the UI thread
    let has_password = secrets::read_off_thread(Secret::Password, &id)
        .await
        .is_some();
    let plugin = app.state::<crate::watch::Plugins>().all().remove(&id);

    MenuFacts {
        has_password,
        plugin_checked: plugin.is_some(),
        plugin: plugin.flatten(),
        too_large: app.state::<crate::watch::Reported>().mentioned(&id),
    }
}

/// One entry of a rail menu, as `shared/web/menus.ts` builds it.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MenuEntry {
    Item {
        id: String,
        label: String,
        checked: Option<bool>,
        disabled: Option<bool>,
    },
    Separator,
    Submenu {
        label: String,
        items: Vec<MenuEntry>,
    },
}

/// More than any rail menu has; a menu past it is refused rather than drawn.
const MAX_MENU_ENTRIES: usize = 64;
const MAX_MENU_LABEL: usize = 120;
const MAX_MENU_ID: usize = 200;

/// An item id is `action:target`: letters and dashes, then ids (uuids) joined by colons. That is all
/// the shell's menus make, and all `lib.rs` hands back to it.
fn is_menu_id(id: &str) -> bool {
    let Some((action, target)) = id.split_once(':') else {
        return false;
    };

    id.len() <= MAX_MENU_ID
        && !action.is_empty()
        && action.chars().all(|c| c.is_ascii_lowercase() || c == '-')
        && !target.is_empty()
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':')
}

fn count_entries(entries: &[MenuEntry]) -> usize {
    entries
        .iter()
        .map(|entry| match entry {
            MenuEntry::Submenu { items, .. } => 1 + count_entries(items),
            _ => 1,
        })
        .sum()
}

fn menu_label(label: &str) -> String {
    shiver_core::text::clamp(label.to_string(), MAX_MENU_LABEL)
}

fn refuse_menu() -> crate::error::Error {
    Core::InvalidInput("That menu cannot be shown.".into()).into()
}

/// The native items for `entries`; a submenu may hold items and separators only.
fn menu_items(
    app: &AppHandle,
    entries: &[MenuEntry],
    nested: bool,
) -> Result<Vec<Box<dyn IsMenuItem<Wry>>>> {
    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::with_capacity(entries.len());

    for entry in entries {
        match entry {
            MenuEntry::Separator => items.push(Box::new(PredefinedMenuItem::separator(app)?)),
            MenuEntry::Item {
                id,
                label,
                checked,
                disabled,
            } => {
                if !is_menu_id(id) {
                    return Err(refuse_menu());
                }

                let enabled = !disabled.unwrap_or(false);

                match checked {
                    Some(checked) => items.push(Box::new(
                        CheckMenuItemBuilder::with_id(id.as_str(), menu_label(label))
                            .checked(*checked)
                            .enabled(enabled)
                            .build(app)?,
                    )),
                    None => items.push(Box::new(
                        MenuItemBuilder::with_id(id.as_str(), menu_label(label))
                            .enabled(enabled)
                            .build(app)?,
                    )),
                }
            }
            MenuEntry::Submenu { .. } if nested => return Err(refuse_menu()),
            MenuEntry::Submenu {
                label,
                items: inner,
            } => {
                let inner = menu_items(app, inner, true)?;
                let refs: Vec<&dyn IsMenuItem<Wry>> = inner.iter().map(AsRef::as_ref).collect();

                items.push(Box::new(
                    SubmenuBuilder::new(app, menu_label(label))
                        .items(&refs)
                        .build()?,
                ));
            }
        }
    }

    Ok(items)
}

/// Shows a rail menu as a native menu at the pointer (an html one would be painted over by the
/// server webview). The shell builds it from the item model both clients share; a chosen item's id
/// comes back to the shell through `lib.rs`.
#[tauri::command]
pub async fn show_menu(app: AppHandle, entries: Vec<MenuEntry>) -> Result<()> {
    if entries.is_empty() || count_entries(&entries) > MAX_MENU_ENTRIES {
        return Err(refuse_menu());
    }

    let window = webviews::main_window(&app)?;
    let items = menu_items(&app, &entries, false)?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(AsRef::as_ref).collect();

    MenuBuilder::new(&app).items(&refs).build()?.popup(window)?;

    Ok(())
}

/* ── showing servers ── */

/// Shows a server, creating its page on first use, and remembers it for next launch.
///
/// Async: creating a webview blocks on the event loop, which a sync command (on the main thread)
/// would deadlock.
#[tauri::command]
pub async fn select_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    let (entry, settings, muted) = page_inputs(&store, &id)?;
    let token = session::token_for_new_page(&app, &entry).await;

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
        let token = session::token(&app, &entry).await;

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
pub async fn exit_dm_split(app: AppHandle) {
    webviews::end_conversation(&app);
}

/// Opens one conversation in the entry's own page, beside Shiver's DM list.
#[tauri::command]
pub async fn open_dm(
    app: AppHandle,
    store: State<'_, Store>,
    entry_id: String,
    name: String,
) -> Result<()> {
    let (entry, settings, muted) = page_inputs(&store, &entry_id)?;
    let token = session::token_for_new_page(&app, &entry).await;

    webviews::show_conversation(
        &app,
        &entry,
        &settings,
        token.as_deref(),
        &muted,
        voice_locked_for(&app, &entry_id),
        &name,
    )?;

    // also stands the entry's socket down, should its page have just been opened
    webviews::trim_pages(&app);

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

/// Makes every site whose links opened without asking ask again.
#[tauri::command]
pub async fn forget_trusted_links(store: State<'_, Store>) -> Result<()> {
    store.update(|registry| {
        registry.settings.trusted_links.clear();

        Ok(())
    })
}

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
    let before = store.registry().settings.start_at_login;
    let saved = store.update(|registry| {
        registry.settings = Settings {
            last_server_id: registry.settings.last_server_id.take(),
            skipped_update: registry.settings.skipped_update.take(),
            trusted_links: std::mem::take(&mut registry.settings.trusted_links),
            ..settings.sanitised()
        };

        Ok(registry.settings.clone())
    })?;

    if saved.start_at_login != before {
        crate::tray::set_start_at_login(&app, saved.start_at_login);
    }

    crate::tray::apply(&app, saved.close_to_tray);
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

/// The bell's count (the feed only, since every entry there has a message to show) and newest entry.
#[tauri::command]
pub fn feed_summary(feed: State<'_, Feed>) -> FeedSummary {
    feed.summary()
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
    crate::watch::mutes_changed(&app, &entry_id);

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
    fn a_menu_item_id_is_an_action_and_its_target() {
        assert!(is_menu_id("open:6f1c2e4a-0b7d-4c1e-9a55-3d2f8e1b7c90"));
        assert!(is_menu_id("move:server-id:folder-id"));
        assert!(!is_menu_id("quit"));
        assert!(!is_menu_id("open:"));
        assert!(!is_menu_id(":id"));
        assert!(!is_menu_id("Open:id"));
        assert!(!is_menu_id("open:id/../x"));
        assert!(!is_menu_id(&format!("open:{}", "a".repeat(MAX_MENU_ID))));
    }

    #[test]
    fn a_menu_counts_what_its_submenus_hold() {
        let entries: Vec<MenuEntry> = serde_json::from_str(
            r#"[{"kind":"item","id":"open:s","label":"Open"},{"kind":"separator"},
                {"kind":"submenu","label":"Notify","items":[{"kind":"item","id":"notify-all:s","label":"All","checked":true}]}]"#,
        )
        .expect("the shell's menu reads");

        assert_eq!(count_entries(&entries), 4);
    }
}
