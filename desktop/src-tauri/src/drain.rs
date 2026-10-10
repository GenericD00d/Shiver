//! Polling the server pages.
//!
//! A Sharkord page has no Tauri IPC, so the core evaluates `__SHIVER_DRAIN__` in it and applies
//! what it returns: every `POLL_INTERVAL` for the page on screen, one still connecting and the one
//! holding the call, every `BACKGROUND_EVERY`th for the rest. A page answers with nothing when
//! nothing changed, except the one being read (on screen in a window that is not minimised: its
//! channel counts as read) and a signed-out one (recovery retries on its reports). A page answers only about itself, and everything it says is
//! treated as a claim: bounded, and never trusted beyond its own entry.

use std::{
    collections::{BTreeSet, HashMap},
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Serialize;
use shiver_core::LockExt;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    feed::{DrainResult, Feed, Route},
    session,
    store::{RegistryStore, Store},
    voice::VoiceState,
    webviews::{self, OVERLAY_WEBVIEW, SHELL_WEBVIEW},
};

const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// Pages in the background are drained on every this many polls (about two seconds).
const BACKGROUND_EVERY: u64 = 3;

/// How long a server may be connecting before the rail calls it offline.
const CONNECT_GRACE: Duration = Duration::from_secs(20);

/// Most notifications taken from one drain; the rest are dropped.
const MAX_NOTIFICATIONS_PER_DRAIN: usize = 50;

const MAX_DRAIN_BYTES: usize = 4 * 1024 * 1024;

const FEED_EVENT: &str = "shiver://feed";
const DM_FAILED_EVENT: &str = "shiver://dm-failed";
const SERVER_READY_EVENT: &str = "shiver://server-ready";
pub const VOICE_EVENT: &str = "shiver://voice";
const SIGNED_OUT_EVENT: &str = "shiver://signed-out";
const STATUS_EVENT: &str = "shiver://status";
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

/// How long a page may say nothing (its document never loaded, so the bridge is not in it) before it
/// is counted as stalled. A page the bridge reports stalled is counted at once.
const STALL_GRACE: Duration = Duration::from_secs(20);

/// Pages that will not recover by themselves: a document that never loaded (the server was down
/// when it opened), or one whose client gave up (Sharkord stops retrying after a few seconds) or
/// never started. Each is asked about with the server's own `/info`, backing off as the core's
/// sockets do, and reloaded once the server answers.
#[derive(Default)]
pub struct Stalls(Mutex<HashMap<String, Stall>>);

struct Stall {
    attempts: u32,
    next: Instant,
    probing: bool,
}

impl Stalls {
    /// Counts the page as stalled from now, unless it already is.
    fn note(&self, entry_id: &str, after: Duration) {
        self.0
            .locked()
            .entry(entry_id.to_string())
            .or_insert_with(|| Stall {
                attempts: 0,
                next: Instant::now() + after,
                probing: false,
            });
    }

    fn clear(&self, entry_id: &str) {
        self.0.locked().remove(entry_id);
    }

    pub fn is_stalled(&self, entry_id: &str) -> bool {
        self.0.locked().contains_key(entry_id)
    }

    /// The stalled pages due a probe, marked as being probed.
    fn due(&self, open: impl Fn(&str) -> bool) -> Vec<String> {
        let mut stalls = self.0.locked();
        let now = Instant::now();

        stalls.retain(|entry_id, _| open(entry_id));
        stalls
            .iter_mut()
            .filter(|(_, stall)| !stall.probing && stall.next <= now)
            .map(|(entry_id, stall)| {
                stall.probing = true;
                entry_id.clone()
            })
            .collect()
    }

    /// A probe ended (and the page was reloaded if the server answered); the next one backs off.
    fn probed(&self, entry_id: &str) {
        if let Some(stall) = self.0.locked().get_mut(entry_id) {
            stall.next = Instant::now() + sharkord_client::retry_delay(stall.attempts, entry_id);
            stall.attempts = stall.attempts.saturating_add(1);
            stall.probing = false;
        }
    }
}

/// The statuses last sent to the rail, so an unchanged set is not re-sent every poll.
#[derive(Default)]
pub struct Broadcast(Mutex<HashMap<String, ServerStatus>>);

pub fn spawn(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);

        for turn in (0..BACKGROUND_EVERY).cycle() {
            ticker.tick().await;
            poll_once(&app, turn == 0);
        }
    });
}

fn poll_once(app: &AppHandle, background_turn: bool) {
    let entry_ids: Vec<String> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    let in_call = app.state::<VoiceState>().holder();

    for entry_id in &entry_ids {
        let Some(webview) = app.get_webview(&webviews::webview_label(entry_id)) else {
            continue;
        };
        let on_screen = crate::badges::is_on_screen(app, entry_id);
        let urgent = on_screen
            || in_call.as_ref() == Some(entry_id)
            || !app.state::<Readiness>().is_ready(entry_id);

        // a full answer each time only while it is being read, so a minimised page answers null
        if urgent || background_turn {
            drain_webview(
                app,
                &webview,
                entry_id.clone(),
                crate::badges::is_being_read(app, entry_id),
            );
        }
    }

    reconcile_voice(app, &entry_ids);
    revive_stalled(app);
    broadcast_statuses(app, &entry_ids);
}

/// Asks each stalled page's server whether it answers, and reloads the page if it does.
fn revive_stalled(app: &AppHandle) {
    for entry_id in app.state::<Stalls>().due(|entry_id| {
        app.get_webview(&webviews::webview_label(entry_id))
            .is_some()
    }) {
        let Some(origin) = app
            .state::<Store>()
            .registry()
            .server(&entry_id)
            .map(|entry| entry.origin.clone())
        else {
            continue;
        };
        let app = app.clone();

        tauri::async_runtime::spawn(async move {
            if shiver_core::probe::fetch_info(&origin).await.is_ok() {
                reload(&app, &entry_id, &origin);
            }

            app.state::<Stalls>().probed(&entry_id);
        });
    }
}

/// Loads a server's page afresh (its bridge, and the session it was made with, come with it).
fn reload(app: &AppHandle, entry_id: &str, origin: &str) {
    let (Some(webview), Ok(url)) = (
        app.get_webview(&webviews::webview_label(entry_id)),
        url::Url::parse(origin),
    ) else {
        return;
    };

    eprintln!("[shiver] {origin} answers again; reloading its page");
    app.state::<Readiness>().disconnected(entry_id);

    if let Err(error) = webview.navigate(url) {
        eprintln!("[shiver] could not reload {origin}'s page: {error}");
    }
}

/// "Try again" on a page that will not recover by itself: reloads it now, and the backoff starts
/// over. Returns whether there was one.
pub fn revive_now(app: &AppHandle, entry_id: &str) -> bool {
    if !app.state::<Stalls>().is_stalled(entry_id) {
        return false;
    }

    if let Some(origin) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .map(|entry| entry.origin.clone())
    {
        app.state::<Stalls>().clear(entry_id);
        reload(app, entry_id, &origin);
    }

    true
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

fn drain_webview(app: &AppHandle, webview: &tauri::Webview, entry_id: String, full: bool) {
    let app = app.clone();

    // null when nothing changed (unless `full`); "absent" while the bridge is not in the page,
    // which past a short grace means its document never loaded
    let script =
        format!("window.__SHIVER_DRAIN__ ? (window.__SHIVER_DRAIN__({full}) || null) : \"absent\"");

    let _ = webview.eval_with_callback(script, move |raw| {
        if raw == "\"absent\"" {
            return app.state::<Stalls>().note(&entry_id, STALL_GRACE);
        }

        if raw.len() > MAX_DRAIN_BYTES {
            return eprintln!(
                "[shiver] {entry_id} answered a drain with {} bytes",
                raw.len()
            );
        }

        let result = match serde_json::from_str::<Option<DrainResult>>(&raw) {
            Ok(Some(result)) => result,
            Ok(None) => return,
            Err(error) => {
                return eprintln!("[shiver] could not read the drain from {entry_id}: {error}")
            }
        };

        apply_in_order(&app, entry_id.clone(), result);
    });
}

/// Applies drains on one thread, in the order the pages answered. Off the UI thread, since applying
/// can write the registry to disk; one thread, since a slow apply finishing after the next one
/// would put older state (a call that ended) back over newer.
fn apply_in_order(app: &AppHandle, entry_id: String, result: DrainResult) {
    static APPLIER: std::sync::OnceLock<std::sync::mpsc::Sender<(String, DrainResult)>> =
        std::sync::OnceLock::new();

    let sender = APPLIER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<(String, DrainResult)>();
        let app = app.clone();

        std::thread::Builder::new()
            .name("shiver-drain".into())
            .spawn(move || {
                for (entry_id, result) in receiver {
                    apply(&app, &entry_id, result);
                }
            })
            .expect("the drain thread should start");

        sender
    });

    let _ = sender.send((entry_id, result));
}

fn apply(app: &AppHandle, entry_id: &str, mut result: DrainResult) {
    let (server_name, account_label, muted, has_identity, origin, notify) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let Some(entry) = registry.server(entry_id) else {
            return;
        };

        (
            entry.name.clone(),
            entry.label(),
            registry.muted_for(entry_id),
            entry.identity.is_some(),
            entry.origin.clone(),
            entry.notify,
        )
    };

    if result.stalled {
        app.state::<Stalls>().note(entry_id, Duration::ZERO);
    } else if result.ready {
        app.state::<Stalls>().clear(entry_id);
    }

    apply_page_state(
        app,
        entry_id,
        &server_name,
        &account_label,
        has_identity,
        &mut result,
    );

    // links only from the page on screen, and then only if the user agrees
    if crate::badges::is_on_screen(app, entry_id) {
        for url in app
            .state::<webviews::Openings>()
            .grant(entry_id, &result.open)
            .iter()
            .filter_map(|address| url::Url::parse(address).ok())
        {
            webviews::ask_to_open(app, entry_id, &server_name, url);
        }
    }

    let feed = app.state::<Feed>();
    let mut changed = false;

    for raw in result
        .notifications
        .into_iter()
        .take(MAX_NOTIFICATIONS_PER_DRAIN)
    {
        // the page applies the level itself (Sharkord knows what mentions the user); the core
        // holds it to "DMs only" at least, which a title is enough to tell
        let is_muted = raw
            .channel_id
            .is_some_and(|channel_id| muted.contains(&channel_id))
            || !notify.allows(raw.is_dm, true);

        if let Some(notification) = feed.push(
            entry_id,
            &server_name,
            Some(&origin),
            (raw, Route::Page),
            is_muted,
        ) {
            crate::notify::announce(app, &notification);
            changed = true;
        }
    }

    if let Some(dms) = result.dms {
        feed.set_dms(entry_id, &server_name, &account_label, Some(&origin), dms);
        changed = true;
    }

    // the rest of what the rail's "Mark all as read" does: Shiver's notifications and missed count
    if result.mark_all_read {
        crate::watch::mark_read(app, entry_id);
        changed |= feed.mark_entry_read(entry_id);
    }

    // Mutes: the plugin's reconciled list replaces ours, then toggles from the channel menu apply on
    // top. One registry write for the lot.
    if result.synced_mutes.is_some() || !result.mutes.is_empty() {
        // from the registry as it is now, inside its update: a toggle made from Shiver's own menu
        // since `muted` was read is kept
        let next_from = |current: Vec<i64>| {
            let mut next: BTreeSet<i64> = result
                .synced_mutes
                .clone()
                .unwrap_or(current)
                .into_iter()
                .collect();

            for mute in &result.mutes {
                let _ = if mute.muted {
                    next.insert(mute.channel_id)
                } else {
                    next.remove(&mute.channel_id)
                };
            }

            shiver_core::model::normalized_mutes(next)
        };
        let stored = app.state::<Store>().update(|registry| {
            let current = registry.muted_for(entry_id);
            let next = next_from(current.clone());

            if next != current {
                registry.set_muted_for(entry_id, next.clone());
            }

            Ok((current, next))
        });

        if let Ok((before, next)) = stored {
            if !result.mutes.is_empty() {
                webviews::push_muted(app, entry_id, &next);
                crate::watch::mutes_changed(app, entry_id);
            }

            changed |= next != before;
        }
    }

    // Only from the page Shiver asked to show a conversation, and named by what Shiver asked for: a
    // page can call its own hook with any text, and this lands in Shiver's DM panel.
    let failed_with = result
        .open_dm_failed
        .then(|| {
            app.state::<webviews::ActiveServer>()
                .conversation_with(entry_id)
        })
        .flatten();

    if let Some(name) = failed_with {
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

    let summary = app.state::<Feed>().summary();

    for label in [SHELL_WEBVIEW, OVERLAY_WEBVIEW, webviews::POPUP_WEBVIEW] {
        let _ = app.emit_to(label, FEED_EVENT, summary);
    }
}

/// The page's own state in a drain: fullscreen, sign-in state, readiness, voice and the channel on
/// screen.
fn apply_page_state(
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

    if app.state::<VoiceState>().report(
        entry_id,
        server_name,
        account_label,
        result.voice.take(),
        crate::badges::is_on_screen(app, entry_id),
    ) {
        let _ = app.emit_to(SHELL_WEBVIEW, VOICE_EVENT, ());
    }

    // not while minimised: what arrives in the channel on screen then is news to the user
    if let Some(channel_id) = result.viewing_channel_id {
        if crate::badges::is_being_read(app, entry_id) {
            crate::badges::channel_viewed(app, entry_id, channel_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stalled_page_is_probed_after_its_grace_once_at_a_time_then_backs_off() {
        let stalls = Stalls::default();
        let open = |_: &str| true;

        stalls.note("a", Duration::from_secs(60));
        assert!(stalls.is_stalled("a"));
        assert!(stalls.due(open).is_empty(), "within its grace");

        stalls.note("b", Duration::ZERO);
        assert_eq!(stalls.due(open), vec!["b".to_string()]);
        assert!(stalls.due(open).is_empty(), "already being probed");

        stalls.probed("b");
        assert!(stalls.due(open).is_empty(), "backing off after a probe");

        // noting again does not restart a stall's clock or its backoff
        stalls.note("b", Duration::ZERO);
        assert!(stalls.due(open).is_empty());

        stalls.clear("b");
        assert!(!stalls.is_stalled("b"));
    }

    #[test]
    fn a_page_that_was_closed_is_no_longer_stalled() {
        let stalls = Stalls::default();

        stalls.note("gone", Duration::ZERO);
        assert!(stalls.due(|_| false).is_empty());
        assert!(!stalls.is_stalled("gone"));
    }
}
