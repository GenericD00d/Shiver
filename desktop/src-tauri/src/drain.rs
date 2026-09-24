//! Polling the server pages.
//!
//! A Sharkord page has no Tauri IPC. Every `POLL_INTERVAL` the core evaluates
//! `__SHIVER_DRAIN__()` in each page and applies what it returns. A page answers only about itself,
//! and everything it says is treated as a claim: bounded, and never trusted beyond its own entry.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Serialize;
use shiver_core::LockExt;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    feed::{DrainResult, Feed},
    session,
    store::{RegistryStore, Store},
    voice::VoiceState,
    webviews::{self, ActiveServer, OVERLAY_WEBVIEW, SHELL_WEBVIEW},
};

const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// How long a server may be connecting before the rail calls it offline.
const CONNECT_GRACE: Duration = Duration::from_secs(20);

/// Most notifications taken from one drain; the rest are dropped.
const MAX_NOTIFICATIONS_PER_DRAIN: usize = 50;

pub const FEED_EVENT: &str = "shiver://feed";
pub const DM_FAILED_EVENT: &str = "shiver://dm-failed";
pub const SERVER_READY_EVENT: &str = "shiver://server-ready";
pub const VOICE_EVENT: &str = "shiver://voice";
pub const SIGNED_OUT_EVENT: &str = "shiver://signed-out";
pub const STATUS_EVENT: &str = "shiver://status";
/// The popup asks the shell to open a message; the shell owns server switching.
pub const OPEN_MESSAGE_EVENT: &str = "shiver://open-message";

/// The rail badge for a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Online,
    Connecting,
    Offline,
}

struct Presence {
    ready: bool,
    /// when Shiver started waiting, which separates connecting from offline
    since: Instant,
}

/// Which servers are connected (via a page or a socket) and how long the others have been trying.
#[derive(Default)]
pub struct Readiness(Mutex<HashMap<String, Presence>>);

impl Readiness {
    pub fn is_ready(&self, entry_id: &str) -> bool {
        self.0
            .locked()
            .get(entry_id)
            .is_some_and(|presence| presence.ready)
    }

    /// Records a report. Returns true when the entry has just become ready.
    pub(crate) fn set(&self, entry_id: &str, ready: bool) -> bool {
        let mut state = self.0.locked();
        let presence = state.entry(entry_id.to_string()).or_insert(Presence {
            ready: false,
            since: Instant::now(),
        });
        let became_ready = ready && !presence.ready;

        presence.ready = ready;

        became_ready
    }

    fn status(&self, entry_id: &str, has_page: bool) -> ServerStatus {
        match self.0.locked().get(entry_id) {
            Some(presence) if presence.ready => ServerStatus::Online,
            Some(presence) if has_page && presence.since.elapsed() < CONNECT_GRACE => {
                ServerStatus::Connecting
            }
            None if has_page => ServerStatus::Connecting,
            _ => ServerStatus::Offline,
        }
    }

    /// A connection dropped: not ready, and the grace clock restarts, so a reconnect reads as
    /// "connecting" rather than "offline".
    pub(crate) fn disconnected(&self, entry_id: &str) {
        self.0.locked().insert(
            entry_id.to_string(),
            Presence {
                ready: false,
                since: Instant::now(),
            },
        );
    }

    pub fn forget_entry(&self, entry_id: &str) {
        self.0.locked().remove(entry_id);
    }
}

/// The statuses last sent to the rail, so an unchanged set is not re-sent every poll.
#[derive(Default)]
pub struct Broadcast(Mutex<HashMap<String, ServerStatus>>);

pub fn spawn(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);

        loop {
            ticker.tick().await;
            poll_once(&app);
        }
    });
}

fn poll_once(app: &AppHandle) {
    let entry_ids: Vec<String> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .map(|entry| entry.id.clone())
        .collect();

    for entry_id in &entry_ids {
        for (label, is_server_page) in [
            (webviews::webview_label(entry_id), true),
            (webviews::dm_webview_label(entry_id), false),
        ] {
            if let Some(webview) = app.get_webview(&label) {
                drain_webview(app, &webview, entry_id.clone(), is_server_page);
            }
        }
    }

    reconcile_voice(app, &entry_ids);
    broadcast_statuses(app, &entry_ids);
}

/// Sends the rail every server's status when it changed. Driven from here because "offline" is
/// reached by nothing happening for long enough.
fn broadcast_statuses(app: &AppHandle, entry_ids: &[String]) {
    let readiness = app.state::<Readiness>();
    let watcher = app.state::<crate::watch::Watcher>();

    let statuses: HashMap<String, ServerStatus> = entry_ids
        .iter()
        .map(|entry_id| {
            let has_page = app
                .get_webview(&webviews::webview_label(entry_id))
                .is_some()
                || watcher.is_watching(entry_id);

            (entry_id.clone(), readiness.status(entry_id, has_page))
        })
        .collect();

    let broadcast = app.state::<Broadcast>();
    let mut last = broadcast.0.locked();

    if *last != statuses {
        let _ = app.emit_to(SHELL_WEBVIEW, STATUS_EVENT, &statuses);
        *last = statuses;
    }
}

/// Pushes voice locks to pages that need them and hangs up any second call that slipped through.
fn reconcile_voice(app: &AppHandle, entry_ids: &[String]) {
    let voice = app.state::<VoiceState>();

    for (entry_id, locked) in voice.pending_locks(entry_ids) {
        webviews::push_voice_lock(app, &entry_id, locked);
    }

    for entry_id in voice.intruders() {
        eprintln!("[shiver] a second voice session started on {entry_id}, leaving it");
        webviews::run_voice_action(app, &entry_id, "leave");
    }
}

fn drain_webview(
    app: &AppHandle,
    webview: &tauri::Webview,
    entry_id: String,
    is_server_page: bool,
) {
    let app = app.clone();

    // null until the bridge has installed itself
    let script = "(window.__SHIVER_DRAIN__ && window.__SHIVER_DRAIN__()) || null";

    let _ = webview.eval_with_callback(script, move |raw| {
        let result = match serde_json::from_str::<Option<DrainResult>>(&raw) {
            Ok(Some(result)) => result,
            Ok(None) => return,
            Err(error) => {
                return eprintln!("[shiver] could not read the drain from {entry_id}: {error}")
            }
        };

        // off the UI thread: applying can write the registry to disk
        let (app, entry_id) = (app.clone(), entry_id.clone());

        tauri::async_runtime::spawn_blocking(move || {
            apply(&app, &entry_id, result, is_server_page)
        });
    });
}

fn apply(app: &AppHandle, entry_id: &str, mut result: DrainResult, is_server_page: bool) {
    let (server_name, account_label, muted, has_identity, origin) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let Some(entry) = registry.server(entry_id) else {
            return;
        };

        (
            entry.name.clone(),
            entry
                .account_label
                .clone()
                .or_else(|| entry.identity.clone())
                .unwrap_or_default(),
            registry.muted_for(entry_id),
            entry.identity.is_some(),
            entry.origin.clone(),
        )
    };

    if is_server_page {
        apply_server_page(
            app,
            entry_id,
            &server_name,
            &account_label,
            has_identity,
            &mut result,
        );
    } else if let Some(channel_id) = result.viewing_channel_id {
        // the conversation view is the only thing that knows which DM is being read
        if app.state::<ActiveServer>().dm_on_screen().as_deref() == Some(entry_id) {
            crate::badges::channel_viewed(app, entry_id, channel_id);
        }
    }

    for url in app
        .state::<webviews::Openings>()
        .grant(entry_id, &result.open)
        .iter()
        .filter_map(|address| url::Url::parse(address).ok())
    {
        webviews::open_in_browser(app, &url);
    }

    let feed = app.state::<Feed>();
    let mut changed = false;

    for raw in result
        .notifications
        .into_iter()
        .take(MAX_NOTIFICATIONS_PER_DRAIN)
    {
        let is_muted = raw
            .channel_id
            .is_some_and(|channel_id| muted.contains(&channel_id));

        changed |= feed.push(entry_id, &server_name, Some(&origin), raw, is_muted);
    }

    if let Some(dms) = result.dms {
        feed.set_dms(entry_id, &server_name, &account_label, Some(&origin), dms);
        changed = true;
    }

    // Mutes: the plugin's reconciled list replaces ours, then toggles from Sharkord's channel menu
    // apply on top. One registry write for the lot.
    if result.synced_mutes.is_some() || !result.mutes.is_empty() {
        let mut next: Vec<i64> = result.synced_mutes.unwrap_or(muted);

        for mute in &result.mutes {
            next.retain(|channel_id| *channel_id != mute.channel_id);

            if mute.muted {
                next.push(mute.channel_id);
            }
        }

        let had_toggles = !result.mutes.is_empty();

        if let Ok(remaining) = app
            .state::<Store>()
            .update(|registry| Ok(registry.set_muted_for(entry_id, next)))
        {
            if had_toggles {
                webviews::push_muted(app, entry_id, &remaining);
            }

            changed = true;
        }
    }

    if let Some(name) = result.open_dm_failed {
        let _ = app.emit_to(
            SHELL_WEBVIEW,
            DM_FAILED_EVENT,
            serde_json::json!({ "name": name }),
        );
    }

    if changed {
        notify_feed_changed(app);
    }
}

/// Tells the rail, bell and popup that the feed moved, and updates the taskbar mark.
pub fn notify_feed_changed<R: tauri::Runtime>(app: &AppHandle<R>) {
    crate::badge::refresh(app);

    for label in [SHELL_WEBVIEW, OVERLAY_WEBVIEW, webviews::POPUP_WEBVIEW] {
        let _ = app.emit_to(label, FEED_EVENT, ());
    }
}

/// The parts of a drain only the server page answers for: fullscreen, sign-in state, readiness,
/// voice and the channel on screen.
fn apply_server_page(
    app: &AppHandle,
    entry_id: &str,
    server_name: &str,
    account_label: &str,
    has_identity: bool,
    result: &mut DrainResult,
) {
    // only the page actually on screen may take the chrome away
    if crate::badges::is_on_screen(app, entry_id) {
        if let Err(error) = webviews::set_page_fullscreen(app, entry_id, result.fullscreen) {
            eprintln!("[shiver] could not follow {entry_id} into fullscreen: {error}");
        }
    }

    let recovery = app.state::<session::Recovery>();
    let recovering = result.signed_out && session::recover(app, entry_id);
    let ready = result.ready && !recovering;

    if result.ready && !result.signed_out {
        recovery.confirmed(entry_id);
    }

    // Shiver cannot sign this one in, so it asks (once) rather than leave the server's login form
    if result.signed_out && !recovering && has_identity && recovery.take_prompt(entry_id) {
        let _ = app.emit_to(
            SHELL_WEBVIEW,
            SIGNED_OUT_EVENT,
            serde_json::json!({ "entryId": entry_id }),
        );
    }

    if app.state::<Readiness>().set(entry_id, ready) {
        let _ = app.emit_to(
            SHELL_WEBVIEW,
            SERVER_READY_EVENT,
            serde_json::json!({ "entryId": entry_id }),
        );
    }

    if app
        .state::<VoiceState>()
        .report(entry_id, server_name, account_label, result.voice.take())
    {
        let _ = app.emit_to(SHELL_WEBVIEW, VOICE_EVENT, ());
    }

    if let Some(channel_id) = result.viewing_channel_id {
        if crate::badges::is_on_screen(app, entry_id) {
            crate::badges::channel_viewed(app, entry_id, channel_id);
        }
    }
}
