//! Draining the server pages.
//!
//! A Sharkord page cannot call into Shiver: it has no Tauri IPC, by design. So the core asks instead,
//! on a timer, by evaluating a drain call in each live webview and reading the result back. The
//! page only ever answers about itself, which is what keeps one server's data out of another's.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use serde::Serialize;

use tauri::{AppHandle, Emitter, Manager};

use crate::{
    feed::{DrainResult, Feed},
    model::MutedChannel,
    session,
    store::{RegistryStore, Store},
    voice::VoiceState,
    webviews::{self, ActiveServer, OVERLAY_WEBVIEW, SHELL_WEBVIEW},
};

/// Fast enough that the bell feels live, slow enough to stay invisible in a profiler.
const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// Event the shell listens on to know the feed moved.
pub const FEED_EVENT: &str = "shiver://feed";

/// Event telling the shell a conversation it asked for could not be opened.
pub const DM_FAILED_EVENT: &str = "shiver://dm-failed";

/// Event telling the shell a server's client finished connecting, so Shiver can stop covering it.
pub const SERVER_READY_EVENT: &str = "shiver://server-ready";

/// Event telling Shiver's own webviews that the voice session changed.
pub const VOICE_EVENT: &str = "shiver://voice";

/// Event asking the shell to collect a server's credentials, because Shiver cannot sign it in.
pub const SIGNED_OUT_EVENT: &str = "shiver://signed-out";

/// Event carrying every server's status, for the badges on the rail.
pub const STATUS_EVENT: &str = "shiver://status";

/// Event asking the shell to take the user to the message they clicked in the feed.
///
/// The feed is drawn in the bell's own webview, which cannot open a server — switching servers is
/// the shell's job, cover and all. So the popup says where the user wants to go and the shell does
/// the going.
pub const OPEN_MESSAGE_EVENT: &str = "shiver://open-message";

/// How long a server may be coming up before Shiver calls it offline.
///
/// Matches the shell's own patience with the server it is waiting on, so the badge and the content
/// area never disagree about whether a server has failed.
const CONNECT_GRACE: Duration = Duration::from_secs(20);

/// What the badge on a server's icon says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    /// the client is up and reporting
    Online,
    /// a page exists and has not answered yet, but has not had long enough to have failed
    Connecting,
    /// no page, or one that has said nothing for `CONNECT_GRACE`
    Offline,
}

struct Presence {
    ready: bool,
    /// when Shiver started waiting on this page, which is what separates connecting from offline
    since: Instant,
}

/// Which servers have a connected client, and how long the others have been trying.
///
/// Shiver shows a loading circle over a server until this says the page has something for the user,
/// and the rail draws its badge from the same state, so the two cannot disagree.
#[derive(Default)]
pub struct Readiness(Mutex<HashMap<String, Presence>>);

impl Readiness {
    fn state(&self) -> MutexGuard<'_, HashMap<String, Presence>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn is_ready(&self, entry_id: &str) -> bool {
        self.state()
            .get(entry_id)
            .is_some_and(|presence| presence.ready)
    }

    /// Records what a page reported. Returns true when it has just become ready.
    ///
    /// `pub(crate)` for `watch.rs`, which reports the same thing about a server it holds a
    /// socket to — a joined connection is that server being reachable just as much as a
    /// page that finished loading is.
    pub(crate) fn set(&self, entry_id: &str, ready: bool) -> bool {
        let mut state = self.state();

        let presence = state.entry(entry_id.to_string()).or_insert(Presence {
            ready: false,
            since: Instant::now(),
        });

        let became_ready = ready && !presence.ready;

        presence.ready = ready;

        became_ready
    }

    /// What the rail should draw for one server.
    ///
    /// `has_page` is asked of the caller rather than looked up here: a server with nothing
    /// reporting for it never had a chance to answer, and is offline however recently Shiver
    /// started waiting. "Nothing reporting" now means neither a webview nor a socket — a
    /// server Shiver deliberately gave no page to is not offline, it is being listened to
    /// another way, and the rail must not claim otherwise.
    fn status(&self, entry_id: &str, has_page: bool) -> ServerStatus {
        let state = self.state();

        let Some(presence) = state.get(entry_id) else {
            return if has_page {
                ServerStatus::Connecting
            } else {
                ServerStatus::Offline
            };
        };

        if presence.ready {
            return ServerStatus::Online;
        }

        if has_page && presence.since.elapsed() < CONNECT_GRACE {
            return ServerStatus::Connecting;
        }

        ServerStatus::Offline
    }

    /// Records that something Shiver was hearing from has stopped, **and restarts the clock**.
    ///
    /// The distinction from `set(id, false)` is the clock, and it is the whole point. `since` is
    /// only ever written when a `Presence` is first inserted, so an entry that has been up for an
    /// hour has an `since` an hour old — and the moment `ready` goes false, `status` finds
    /// `elapsed() > CONNECT_GRACE` and calls it **offline instantly**, with no grace at all.
    ///
    /// That is fine for a page, which re-reports every 750ms and is back within a tick. It is wrong
    /// for a socket: `watch.rs` waits `RETRY_AFTER` before trying again, so a single dropped
    /// connection put "not responding" on the rail for the full thirty seconds, for a server that
    /// was about to come back and had never stopped being reachable. That is what made the warning
    /// look like it was about something other than the server's health.
    ///
    /// With the clock restarted, a drop reads as connecting for `CONNECT_GRACE` and only becomes
    /// offline if it is still down when that runs out — which is the question the rail is asking.
    pub(crate) fn disconnected(&self, entry_id: &str) {
        let mut state = self.state();

        state.insert(
            entry_id.to_string(),
            Presence {
                ready: false,
                since: Instant::now(),
            },
        );
    }

    /// A closed page is not a connected one, so its next open waits for the client again — and
    /// waits from now, which stops a rebuilt page inheriting the old one's patience.
    pub fn forget_entry(&self, entry_id: &str) {
        self.state().remove(entry_id);
    }
}

/// The last statuses handed to the rail, so an unchanged set is not re-sent every poll.
#[derive(Default)]
pub struct Broadcast(Mutex<HashMap<String, ServerStatus>>);

/// How many links one page may have opened in the browser recently, per rail entry.
///
/// A page hands Shiver the addresses it wants opened and Shiver opens them, which a hostile page can
/// turn into a tab flood or a stream of download prompts by queueing hundreds at once. The scheme is
/// already restricted to http and https (`webviews::open_externally`) — what was missing is a
/// number.
///
/// Two limits, because either alone is not enough: a per-poll cap on its own still allows
/// `OPEN_PER_TICK` every 750 ms, which is hundreds of tabs a minute.
#[derive(Default)]
pub struct Openings(Mutex<HashMap<String, Vec<Instant>>>);

/// The most links one drain of one page may open. A click opens one, so this is generous.
const OPEN_PER_TICK: usize = 5;

/// And the most it may open within `OPEN_WINDOW`, so a page cannot simply queue again next poll.
const OPEN_PER_WINDOW: usize = 10;
const OPEN_WINDOW: Duration = Duration::from_secs(10);

impl Openings {
    /// How many more this entry may open now, forgetting whatever has aged out of the window.
    fn allowance(&self, entry_id: &str) -> usize {
        let mut seen = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let recent = seen.entry(entry_id.to_string()).or_default();

        recent.retain(|at| at.elapsed() < OPEN_WINDOW);

        OPEN_PER_WINDOW
            .saturating_sub(recent.len())
            .min(OPEN_PER_TICK)
    }

    fn record(&self, entry_id: &str) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(entry_id.to_string())
            .or_default()
            .push(Instant::now());
    }

    pub fn forget_entry(&self, entry_id: &str) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(entry_id);
    }
}

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
    let entries = {
        let store = app.state::<Store>();
        let registry = store.registry();

        registry.servers.clone()
    };

    for entry in &entries {
        // both pages for an entry: the server view reports the feed, the conversation view reports
        // only whether a DM Shiver asked for could be opened
        let labels = [
            (webviews::webview_label(&entry.id), true),
            (webviews::dm_webview_label(&entry.id), false),
        ];

        for (label, is_server_page) in labels {
            let Some(webview) = app.get_webview(&label) else {
                continue;
            };

            drain_webview(app, &webview, entry.id.clone(), is_server_page);
        }
    }

    reconcile_voice(
        app,
        &entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
    );

    broadcast_statuses(app, &entries);
}

/// Tells the rail how every server is doing, when that has changed.
///
/// Driven from here rather than polled by the shell because two of the three states are not events:
/// a server goes from connecting to offline by *nothing* happening for long enough, and there is
/// nothing for a page to report at the moment it stops being worth waiting for.
fn broadcast_statuses(app: &AppHandle, entries: &[crate::model::ServerEntry]) {
    let readiness = app.state::<Readiness>();

    let statuses: HashMap<String, ServerStatus> = entries
        .iter()
        .map(|entry| {
            let has_page = app
                .get_webview(&webviews::webview_label(&entry.id))
                .is_some()
                || app.state::<crate::watch::Watcher>().is_watching(&entry.id);

            (entry.id.clone(), readiness.status(&entry.id, has_page))
        })
        .collect();

    let broadcast = app.state::<Broadcast>();
    let mut last = broadcast
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if *last == statuses {
        return;
    }

    *last = statuses.clone();

    let _ = app.emit_to(SHELL_WEBVIEW, STATUS_EVENT, statuses);
}

/// Keeps one call across every server.
///
/// The lock is what normally prevents a second one, so it is pushed to every page that is not the
/// holder. Anything already in a call it should not be in is told to hang up, which covers a join
/// that beat the lock there.
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

    // the page returns null until the bridge has installed itself, which is normal for the first
    // few ticks after a webview opens
    let script = "(window.__SHIVER_DRAIN__ && window.__SHIVER_DRAIN__()) || null";

    let _ = webview.eval_with_callback(script, move |raw| {
        match serde_json::from_str::<Option<DrainResult>>(&raw) {
            Ok(Some(result)) => apply(&app, &entry_id, result, is_server_page),
            Ok(None) => {}
            // a page that answers with something Shiver cannot read loses the whole tick, so it is
            // worth saying so: this is silent otherwise, and it hid a timestamp that arrived as a
            // float for as long as it took to notice the cache was never written
            Err(error) => eprintln!("[shiver] could not read the drain from {entry_id}: {error}"),
        }
    });
}

fn apply(app: &AppHandle, entry_id: &str, mut result: DrainResult, is_server_page: bool) {
    let (server_name, account_label, muted, has_identity, origin) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let Some(entry) = registry.servers.iter().find(|server| server.id == entry_id) else {
            return;
        };

        let muted: Vec<i64> = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == entry_id)
            .map(|muted| muted.channel_id)
            .collect();

        let label = entry
            .account_label
            .clone()
            .or_else(|| entry.identity.clone())
            .unwrap_or_default();

        (
            entry.name.clone(),
            label,
            muted,
            entry.identity.is_some(),
            entry.origin.clone(),
        )
    };

    // The conversation view is a second client for the same server. It answers only about the DM it
    // was asked to open, so everything an entry reports *once* is taken from the server page alone
    // — with one exception, below: which conversation it is showing is the one thing only it knows.
    if !is_server_page {
        mark_read_conversation_read(app, entry_id, result.viewing_channel_id);
    }

    if is_server_page {
        let page = ServerPageReport {
            ready: result.ready,
            signed_out: result.signed_out,
            voice: result.voice.take(),
            viewing_channel_id: result.viewing_channel_id,
            fullscreen: result.fullscreen,
        };

        apply_server_page(
            app,
            entry_id,
            &server_name,
            &account_label,
            has_identity,
            page,
        );
    }

    // Links the page could not open itself. Sharkord opens files and outside links in a new window,
    // which Shiver has nowhere to put and the webview refuses — so the bridge catches the click and
    // leaves the address here. The browser is where they belong: it is where every other link out
    // of Sharkord goes, and it is the thing that knows how to save a file.
    //
    // Rationed, because Shiver is doing this on a page's word. A person clicks one link; a page that
    // asks for hundreds is doing something else, and the browser would answer every one of them
    // with a tab or a download prompt. See `Openings`.
    let queued = std::mem::take(&mut result.open);
    let allowance = app.state::<Openings>().allowance(entry_id);

    if queued.len() > allowance {
        // said out loud: dropping what a page asked for is exactly the kind of thing that reads as
        // a broken link rather than a limit being applied
        eprintln!(
            "[shiver] {entry_id} asked to open {} addresses, opening {allowance} — the rest dropped",
            queued.len()
        );
    }

    for address in queued.into_iter().take(allowance) {
        match url::Url::parse(&address) {
            Ok(url) => {
                app.state::<Openings>().record(entry_id);
                webviews::open_externally(app, &url);
            }
            Err(error) => eprintln!("[shiver] {entry_id} asked to open {address}: {error}"),
        }
    }

    let feed = app.state::<Feed>();
    let mut changed = false;

    for raw in result.notifications {
        let is_muted = raw
            .channel_id
            .is_some_and(|channel_id| muted.contains(&channel_id));

        if feed.push(entry_id, &server_name, Some(&origin), raw, is_muted) {
            changed = true;
        }
    }

    if let Some(dms) = result.dms {
        feed.set_dms(entry_id, &server_name, &account_label, Some(&origin), dms);

        changed = true;
    }

    // the plugin has reconciled this device's mutes with the server's copy, so adopt the result
    if let Some(synced) = result.synced_mutes {
        let store = app.state::<Store>();

        let _ = store.update(|registry| {
            registry.muted.retain(|entry| entry.entry_id != entry_id);

            for channel_id in &synced {
                registry.muted.push(MutedChannel {
                    entry_id: entry_id.to_string(),
                    channel_id: *channel_id,
                });
            }

            Ok(())
        });

        changed = true;
    }

    // mutes toggled from Sharkord's own channel context menu arrive the same way notifications do
    for mute in result.mutes {
        let store = app.state::<Store>();

        let applied = store.update(|registry| {
            registry.muted.retain(|entry| {
                !(entry.entry_id == entry_id && entry.channel_id == mute.channel_id)
            });

            if mute.muted {
                registry.muted.push(MutedChannel {
                    entry_id: entry_id.to_string(),
                    channel_id: mute.channel_id,
                });
            }

            Ok(registry
                .muted
                .iter()
                .filter(|entry| entry.entry_id == entry_id)
                .map(|entry| entry.channel_id)
                .collect::<Vec<_>>())
        });

        if let Ok(remaining) = applied {
            webviews::push_muted(app, entry_id, &remaining);

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

/// Tells every Shiver webview that shows the feed that it moved.
///
/// All three need it and each is a separate page: the shell draws the rail badges and the DM list,
/// the bell draws the count, and the popup draws the feed itself.
pub fn notify_feed_changed<R: tauri::Runtime>(app: &AppHandle<R>) {
    // the taskbar follows the same signal the bell does, so there is one answer to "is anything
    // unread" rather than two that can disagree
    crate::badge::refresh(app);

    let _ = app.emit_to(SHELL_WEBVIEW, FEED_EVENT, ());
    let _ = app.emit_to(OVERLAY_WEBVIEW, FEED_EVENT, ());
    let _ = app.emit_to(webviews::POPUP_WEBVIEW, FEED_EVENT, ());
}

/// The parts of a drain only the server page answers for.
struct ServerPageReport {
    ready: bool,
    signed_out: bool,
    voice: Option<crate::voice::VoiceSnapshot>,
    viewing_channel_id: Option<i64>,
    fullscreen: bool,
}

fn apply_server_page(
    app: &AppHandle,
    entry_id: &str,
    server_name: &str,
    account_label: &str,
    has_identity: bool,
    report: ServerPageReport,
) {
    // A page asking for credentials is "usable", and Shiver still does not want to show it: it
    // promised the user would never meet a login form, and it may be able to keep that promise by
    // signing back in. So the circle stays up while it tries, and comes down either when the
    // rebuilt page connects or when Shiver runs out of ways to avoid the form.
    // Only the server being *shown* can be filling the screen. A preloaded page in the background
    // reporting fullscreen would otherwise resize a webview nobody is looking at, and take the bell
    // away from the server that is.
    // `showing_server` as well as `get`: `get` keeps naming a server while Shiver has settings or
    // the DM inbox over the top of it, and hiding the bell for a page nobody can see is not the job.
    let active = app.state::<crate::webviews::ActiveServer>();

    if active.get().as_deref() == Some(entry_id) && active.showing_server() {
        if let Err(error) = crate::webviews::set_page_fullscreen(app, entry_id, report.fullscreen) {
            eprintln!("[shiver] could not follow {entry_id} into fullscreen: {error}");
        }
    }

    let recovering = report.signed_out && session::recover(app, entry_id);
    let ready = report.ready && !recovering;

    // Shiver cannot sign this one in and the page is asking for credentials, which is the login
    // screen the whole sign-in design exists to avoid. So Shiver asks instead: it can put what it is
    // given in the keychain, which is what makes the *next* time seamless — and what a password
    // typed into the server's own page never does, because Shiver never sees it.
    if report.signed_out
        && !recovering
        && has_identity
        && app.state::<session::Recovery>().take_prompt(entry_id)
    {
        let _ = app.emit_to(
            SHELL_WEBVIEW,
            SIGNED_OUT_EVENT,
            serde_json::json!({ "entryId": entry_id }),
        );
    }

    // the shell is waiting on this to take its loading circle down, so it is told the moment the
    // page first reports itself usable rather than on the next poll
    if app.state::<Readiness>().set(entry_id, ready) {
        let _ = app.emit_to(
            SHELL_WEBVIEW,
            SERVER_READY_EVENT,
            serde_json::json!({ "entryId": entry_id }),
        );
    }

    if app
        .state::<VoiceState>()
        .report(entry_id, server_name, account_label, report.voice)
    {
        let _ = app.emit_to(SHELL_WEBVIEW, VOICE_EVENT, ());
    }

    mark_viewed_channel_read(app, entry_id, report.viewing_channel_id);
}

/// Treats an open conversation as read.
///
/// Reading a direct message did not clear its notification, because the only thing that cleared
/// anything was `mark_viewed_channel_read` below, and that is fed by the *server page*. A DM opens
/// in its own view beside the inbox; the server page carries on reporting whatever channel it was
/// left on, so nothing ever named the conversation the user was actually reading and its entry sat
/// unread in the bell for good.
///
/// Gated on that view being on screen, for the reason spelled out on `dm_on_screen`: a conversation
/// view can exist without being looked at.
fn mark_read_conversation_read<R: tauri::Runtime>(
    app: &AppHandle<R>,
    entry_id: &str,
    channel_id: Option<i64>,
) {
    let Some(channel_id) = channel_id else {
        return;
    };

    if app.state::<ActiveServer>().dm_on_screen().as_deref() != Some(entry_id) {
        return;
    }

    crate::badges::channel_viewed(app, entry_id, channel_id);
}

/// Treats the channel on screen as read, so its badge falls away while the rest of the server's
/// unread stays.
///
/// Gated on the server actually being *visible*, not merely selected. A hidden page still reports a
/// selected channel, and Sharkord raises a notification for the selected channel exactly when its
/// window is hidden — so acting on the report alone would quietly eat the badges for whatever
/// channel the user happened to leave open while they were looking at something else.
fn mark_viewed_channel_read<R: tauri::Runtime>(
    app: &AppHandle<R>,
    entry_id: &str,
    channel_id: Option<i64>,
) {
    let Some(channel_id) = channel_id else {
        return;
    };

    if !crate::badges::is_on_screen(app, entry_id) {
        return;
    }

    crate::badges::channel_viewed(app, entry_id, channel_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One click at a time is what a person does, and it must stay unaffected by the ration.
    #[test]
    fn an_ordinary_click_is_never_rationed() {
        let openings = Openings::default();

        for _ in 0..OPEN_PER_WINDOW {
            assert!(openings.allowance("a") >= 1);
            openings.record("a");
        }
    }

    /// A page queueing hundreds gets the per-poll cap, not the queue.
    #[test]
    fn one_drain_opens_at_most_the_per_tick_cap() {
        assert_eq!(Openings::default().allowance("a"), OPEN_PER_TICK);
    }

    /// And it cannot simply queue again on the next poll: the window is what stops that, since the
    /// drain comes round every 750 ms and the cap alone would allow hundreds a minute.
    #[test]
    fn queueing_again_next_poll_runs_out_of_window() {
        let openings = Openings::default();

        for _ in 0..OPEN_PER_WINDOW {
            openings.record("a");
        }

        assert_eq!(openings.allowance("a"), 0);
    }

    /// Rationed per rail entry, so one page misbehaving does not close the door on another server.
    #[test]
    fn one_server_spending_its_allowance_does_not_spend_anothers() {
        let openings = Openings::default();

        for _ in 0..OPEN_PER_WINDOW {
            openings.record("a");
        }

        assert_eq!(openings.allowance("a"), 0);
        assert_eq!(openings.allowance("b"), OPEN_PER_TICK);
    }
}
