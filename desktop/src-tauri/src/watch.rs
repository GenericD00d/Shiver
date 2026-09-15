//! Shiver's own connection to the servers it is not showing.
//!
//! Desktop used to learn everything about a server by keeping that server's client alive in a
//! webview and evaluating script in it every 750 ms (`drain.rs`). That works, and it is still how
//! the server on screen is read — a page knows things no socket can, like what is fullscreen or
//! which call the user is in. What it is not is cheap: a WebView2 instance with a full Sharkord
//! client in it costs on the order of a hundred megabytes, and people are in dozens of servers.
//!
//! Mobile never had the option. Android gives a window one webview, so the servers not on screen
//! had to be spoken to directly, and `shiver-sharkord` is that conversation. This module is desktop
//! doing the same thing for the opposite reason: not because it cannot open a page, but because it
//! should not open thirty.
//!
//! **The split is by what only a page can answer.** A socket gives messages, conversations and
//! whether the server is reachable. It cannot give fullscreen state, voice state, the channel being
//! viewed, or a mute toggled from Sharkord's own context menu — and every one of those is about the
//! server the user is looking at, which still has its page. So nothing is lost by the server that is
//! not on screen having no page, which is the whole argument for this.
//!
//! One thing deliberately *not* carried over from mobile: the unread baseline. Mobile counts unread
//! from the server's own read states, so it needs a floor to measure against or a public server's
//! entire backlog reads as unread. Desktop's badge has always counted unread entries in the feed,
//! and `Event::Posted` is exactly "a message arrived just now" — so the badge keeps meaning what it
//! already meant, and there is no baseline to get wrong.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use tauri::{async_runtime::JoinHandle, AppHandle, Manager, Runtime};

use shiver_sharkord as sharkord;

use crate::{
    commands, drain,
    drain::Readiness,
    feed::{DmChannel, Feed, RawNotification},
    model::ServerEntry,
    secrets::{self, Secret},
    store::Store,
    webviews,
};

/// How long to wait before trying a server that just failed. Matches mobile's, for the same reason:
/// long enough not to hammer a server that is down, short enough to catch up quickly.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// What each server was holding unread **when Shiver was last away**, above the floor.
///
/// This is the half of a badge the feed cannot know: messages that arrived while Shiver was closed
/// leave no notification behind, because nothing was running to take one. Everything that arrives
/// while Shiver *is* running goes to the feed instead, and the rail adds the two.
///
/// **It is counted once per server per run and never recomputed upward.** That is what stops the
/// same message being counted twice: a socket that drops and reconnects re-reads the server's unread
/// — which still includes everything already sitting in the feed — so recomputing on every join
/// would double each one. It only ever goes *down* from there, when a channel is read.
///
/// Trying to make this the whole badge was a mistake worth remembering: it made the rail depend on
/// read-state deltas arriving, on the floor being right, and on the plugin's shared floor being
/// sane, where the feed depended only on a message arriving. The badge disappeared entirely.
///
/// Counted once per connection, not per message, and kept apart from the `Feed` on purpose: the
/// feed holds messages Shiver actually saw, and these are messages it did not — they arrived while
/// it was closed, and nothing here has their author or their text. Only their number is knowable
/// without asking the server for history.
///
/// The rail badge and the taskbar dot add the two together, so a restart shows what was missed
/// rather than starting from zero.
#[derive(Default)]
pub struct Missed(Mutex<HashMap<String, usize>>);

/// What each watched server currently reports unread, per channel.
///
/// Kept live rather than read once, because a read on another of this user's devices arrives as an
/// event and has to be applied against something. The page cannot supply this — Sharkord's plugin
/// store does not expose `readStatesMap` — so the socket's own running copy is the only one there
/// is.
#[derive(Default)]
pub struct ReadStates(Mutex<HashMap<String, HashMap<i64, u32>>>);

impl ReadStates {
    pub(crate) fn set(&self, entry_id: &str, states: HashMap<i64, u32>) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(entry_id.to_string(), states);
    }

    /// The map itself, for the counting that is tested without a runtime. Not `inner`: that is
    /// already a method on tauri's `State`, and the wrong one would be picked silently.
    pub(crate) fn map(&self) -> &Mutex<HashMap<String, HashMap<i64, u32>>> {
        &self.0
    }
}

/// Treats a channel the user is looking at as read, and works the badge out again.
///
/// **This is how a badge comes down while a server has a page.** A page means no socket, and the
/// socket is what would otherwise report the read — so without this, opening a server and reading
/// the message that caused its badge left the badge standing until the user navigated away and the
/// connection came back. Which is exactly what it looked like: the message plainly read, and Shiver
/// still insisting.
///
/// Zeroing the channel is not a guess. Sharkord marks a channel read as a side effect of showing it
/// (`setSelectedChannelId` -> `markChannelAsRead`), so by the time a page reports viewing one, the
/// server already agrees.
pub fn channel_read<R: Runtime>(app: &AppHandle<R>, entry_id: &str, channel_id: i64) {
    let (floor, muted) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        (
            registry.baselines.get(entry_id).cloned().unwrap_or_default(),
            registry
                .muted
                .iter()
                .filter(|muted| muted.entry_id == entry_id)
                .map(|muted| muted.channel_id)
                .collect::<Vec<_>>(),
        )
    };

    let changed = recount_without(
        app.state::<ReadStates>().map(),
        app.state::<Missed>().map(),
        entry_id,
        channel_id,
        &floor,
        &muted,
    );

    if changed {
        drain::notify_feed_changed(app);
    }
}

/// Treats one channel as read and works the server's badge out again. Returns whether it moved.
///
/// Takes the two state maps rather than an `AppHandle`, so the thing that has been wrong four times
/// — whether reading a channel actually brings the number down — can be tested without a runtime.
pub(crate) fn recount_without(
    read_states: &Mutex<HashMap<String, HashMap<i64, u32>>>,
    missed: &Mutex<HashMap<String, usize>>,
    entry_id: &str,
    channel_id: i64,
    floor: &HashMap<i64, u32>,
    muted: &[i64],
) -> bool {
    let states = {
        let mut held = read_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let Some(states) = held.get_mut(entry_id) else {
            return false;
        };

        // nothing was unread there, so nothing about the badge has changed
        if states.remove(&channel_id).is_none() {
            return false;
        }

        states.clone()
    };

    let count = missed_since(&states, floor, muted);

    let mut held = missed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if count == 0 {
        held.remove(entry_id);
    } else {
        held.insert(entry_id.to_string(), count);
    }

    true
}

impl Missed {
    pub fn counts(&self) -> HashMap<String, usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn set(&self, entry_id: &str, count: usize) {
        let mut state = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if count == 0 {
            state.remove(entry_id);
        } else {
            state.insert(entry_id.to_string(), count);
        }
    }

    pub(crate) fn map(&self) -> &Mutex<HashMap<String, usize>> {
        &self.0
    }

    /// Opening a server settles what was missed on it, the same way it settles the feed.
    fn clear(&self, entry_id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(entry_id)
            .is_some()
    }
}

/// Which servers have Shiver's companion plugin, and which version.
///
/// **A key is only present once Shiver has connected to that server**, and the distinction matters:
/// a missing key means "not asked yet", `None` means "asked, and it is not installed". Showing the
/// two the same way would report every server Shiver has not reached as missing the plugin.
///
/// It cannot be asked any earlier. `/info` does not mention plugins, `plugins.get` needs
/// `MANAGE_PLUGINS` and so answers only an admin, and the one place an ordinary member is told is
/// the `joinServer` payload — which needs a session. So this arrives with the first connection and
/// not before it.
#[derive(Default)]
pub struct Plugins(Mutex<HashMap<String, Option<String>>>);

impl Plugins {
    pub fn all(&self) -> HashMap<String, Option<String>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set(&self, entry_id: &str, version: Option<String>) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(entry_id.to_string(), version);
    }
}

/// The connections Shiver is holding, by rail entry.
#[derive(Default)]
pub struct Watcher(Mutex<HashMap<String, JoinHandle<()>>>);

impl Watcher {
    fn state(&self) -> MutexGuard<'_, HashMap<String, JoinHandle<()>>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether this entry is being watched from the core.
    ///
    /// Asked by the rail, which draws the same badge whether a server is reporting through a page
    /// or through a socket — a user has no reason to care which, and a server that read "offline"
    /// because Shiver chose not to give it a webview would be a lie.
    pub fn is_watching(&self, entry_id: &str) -> bool {
        self.state().contains_key(entry_id)
    }
}

/// Brings the set of live sockets in line with what Shiver should be holding.
///
/// Idempotent, so every caller can simply say "something changed" rather than working out what.
/// Called when a server is opened or closed, added or removed, or signed in.
pub fn sync(app: &AppHandle) {
    let entries: Vec<ServerEntry> = app.state::<Store>().registry().servers.clone();

    let wanted: Vec<ServerEntry> = entries
        .into_iter()
        // A server with its own page open reports for itself, and two things reporting the same
        // messages would put each of them in the feed twice.
        .filter(|entry| {
            app.get_webview(&webviews::webview_label(&entry.id))
                .is_none()
        })
        // Shiver cannot open a socket for a server it has no account on. That server stays in the
        // rail and stays openable; it simply has nothing to say until somebody signs it in.
        .filter(|entry| entry.identity.is_some())
        .collect();

    let unwanted: Vec<String> = {
        let held = app.state::<Watcher>();
        let state = held.state();

        state
            .keys()
            .filter(|entry_id| !wanted.iter().any(|entry| entry.id == **entry_id))
            .cloned()
            .collect()
    };

    for entry_id in unwanted {
        let task = app.state::<Watcher>().state().remove(&entry_id);

        if let Some(task) = task {
            task.abort();
        }

        // Whatever this socket had said about the server, it is not saying it any more. The page
        // taking over answers for itself from its first drain.
        app.state::<Readiness>().forget_entry(&entry_id);
    }

    for entry in wanted {
        let entry_id = entry.id.clone();

        if app.state::<Watcher>().is_watching(&entry_id) {
            continue;
        }

        let handle = app.clone();
        let id = entry_id.clone();

        let task = tauri::async_runtime::spawn(async move {
            watch(handle, entry, id).await;
        });

        app.state::<Watcher>().state().insert(entry_id, task);
    }
}

/// Holds one server's connection open, reconnecting for as long as Shiver still wants it.
async fn watch(app: AppHandle, entry: ServerEntry, entry_id: String) {
    let origin = entry.origin.clone();

    loop {
        // Asked for every attempt rather than once: this is what refreshes an expiring session from
        // the stored password, and a watch that ran for a week would otherwise be holding a token
        // that expired on day seven.
        let Some(token) = commands::ensure_session(&entry).await else {
            eprintln!("[shiver] no session for {origin}, so it is not being watched");

            // Nothing here changes on its own — it needs the user — so this task ends rather than
            // retrying every thirty seconds forever. `sync` starts a new one after a sign-in.
            forget(&app, &entry_id);

            return;
        };

        // `false` is the bounded frame limit, and desktop has no way to lift it. Mobile does —
        // a per-server "Trust" button that appears once a server has actually tripped the limit —
        // and that pair, the reported problem and the button that answers it, is a piece of UI
        // this change did not bring across. Until it does, a server that sends more than
        // `MAX_FRAME` in one message is logged and retried rather than watched.
        match sharkord::open(&origin, &token, false).await {
            Ok(mut session) => {
                eprintln!(
                    "[shiver] watching {origin} from the core, {} channels",
                    session.joined.read_states.len()
                );

                // A joined socket is the server being reachable, which is what the rail's dot
                // means. Set here rather than inferred from the task existing: the task exists
                // while it is failing to connect, too.
                //
                // `sync` aborts this task and clears the readiness when a page takes over, and the
                // two can in principle cross: `abort` only takes effect at the next await, so a
                // join landing in that instant can set readiness for a server whose page has just
                // been built. The cost is that `prepare_server` calls that page ready one time too
                // early and the user watches the client boot for a second, which is why this is
                // written down rather than guarded — the guard would be a lock held across a
                // network round trip.
                app.state::<Readiness>().set(&entry_id, true);

                publish_dms(&app, &entry_id, &session.joined);
                count_missed(
                    &app,
                    &entry_id,
                    &session.joined.read_states,
                    session.joined.shared_floor.clone(),
                );

                app.state::<Plugins>()
                    .set(&entry_id, session.joined.plugin_version.clone());
                app.state::<ReadStates>()
                    .set(&entry_id, session.joined.read_states.clone());
                drain::notify_feed_changed(&app);

                // kept live so a read reported by another device can be applied against it
                let mut read_states = session.joined.read_states.clone();

                while let Some(event) = session.next_event().await {
                    match event {
                        sharkord::Event::Posted(message) => {
                            announce(&app, &entry_id, &session.joined, &message);
                        }
                        // A new message. Tracked so a later read has something to subtract from,
                        // but **not** added to `Missed`: Shiver is running, so this message is
                        // already in the feed, and the rail adds the two.
                        sharkord::Event::Unread { channel_id, delta } => {
                            sharkord::apply_delta(&mut read_states, channel_id, delta);
                            app.state::<ReadStates>().set(&entry_id, read_states.clone());
                        }
                        // The user read that channel somewhere else. This is the one event that
                        // may lower what Shiver is claiming, so the count is worked out again.
                        sharkord::Event::UnreadSet { channel_id, count } => {
                            sharkord::set_unread(&mut read_states, channel_id, count);
                            app.state::<ReadStates>().set(&entry_id, read_states.clone());

                            // **The server has told Shiver this channel was read**, wherever that
                            // happened — this machine, the phone, or a browser. It is the only
                            // signal there is for the server the user is looking at, whose page
                            // cannot see read states and whose `selectedChannelId` does not move
                            // when a conversation is opened.
                            //
                            // Nothing left unread in it means the notifications for it are settled
                            // too, which is what actually takes the badge down: this one lives in
                            // the feed, not in the count below.
                            if count == 0 {
                                crate::badges::channel_viewed(&app, &entry_id, channel_id);
                            }

                            count_missed(
                                &app,
                                &entry_id,
                                &read_states,
                                session.joined.shared_floor.clone(),
                            );

                            drain::notify_feed_changed(&app);
                        }
                    }
                }

                eprintln!("[shiver] {origin} closed the connection");
            }
            // A token the server rejects will be rejected again in thirty seconds and every thirty
            // seconds after that. `ensure_session` above is the thing that can do something about
            // it, so this drops the stored session and goes back round to let it.
            Err(sharkord::Error::Refused(reason)) => {
                eprintln!("[shiver] {origin} refused Shiver's session ({reason})");

                let _ = secrets::forget(Secret::Session, &entry_id);
            }
            // The one failure that will not come right by itself: the next attempt asks the same
            // question and gets the same oversized answer. Retried anyway — a server that trims
            // what it sends fixes it without anyone restarting anything — but said out loud,
            // because that server has otherwise quietly stopped existing.
            Err(sharkord::Error::TooLarge { size, max }) => {
                eprintln!(
                    "[shiver] {origin} sent {size} bytes in one message and Shiver accepts {max}, so it is not being watched"
                );
            }
            Err(error) => eprintln!("[shiver] could not watch {origin}: {error}"),
        }

        // Shiver is not hearing from this server now — but it is about to try again, and the rail
        // should say "connecting" for that window rather than "not responding" the instant a socket
        // blips. `disconnected` restarts the grace clock; a plain `set(false)` would not, and a
        // server that had been up for an hour would be called offline immediately. See its doc.
        app.state::<Readiness>().disconnected(&entry_id);

        tokio::time::sleep(RETRY_AFTER).await;
    }
}

/// Drops every trace of a watch, for a task that is ending rather than retrying.
fn forget(app: &AppHandle, entry_id: &str) {
    app.state::<Watcher>().state().remove(entry_id);
    app.state::<Readiness>().forget_entry(entry_id);
}

/// Hands a server's page the floor, so this user's other devices measure from the same place.
///
/// **It does not move the floor, and it does not clear the count.** Both of those were here, and
/// both were wrong: opening a server dropped its floor and zeroed its badge, so walking into a
/// server with five unread lost all five whether or not anything had been read.
///
/// The floor answers one question — what was already sitting unread the first time Shiver saw this
/// server — and the answer does not change by looking at it. What brings a badge down is Sharkord's
/// own read state falling as channels are actually read, which is reported back either on the next
/// connection or, for a read on another device, by `Event::UnreadSet`.
///
/// Publishing the same floor on every open is deliberate and idempotent: it costs one write and it
/// is how a server that gains the plugin later catches up without anything having to notice.
pub fn publish_floor(app: &AppHandle, entry_id: &str) {
    let id = entry_id.to_string();
    let floor = app.state::<Store>().registry().baselines.get(&id).cloned();

    if let Some(floor) = floor {
        store_shared_floor(app, entry_id, &floor);
    }
}

/// Settles a server outright, for the rail menu's "Mark all as read".
///
/// Distinct from opening one, which settles nothing. This is the user saying so, and it is paired
/// with `webviews::mark_all_read`, which marks the channels read on the server itself — so the
/// count would fall on the next connection anyway. Clearing it here just means not waiting.
pub fn mark_read(app: &AppHandle, entry_id: &str) {
    if app.state::<Missed>().clear(entry_id) {
        drain::notify_feed_changed(app);
    }
}

/// Asks a server's own page to store the floor where this user's other devices will find it.
///
/// Evaluated into the page rather than sent from the socket, because storing it is a mutation and
/// this module's connection is read-only by design. The page retries nothing and reports nothing:
/// a server with no companion plugin answers NOT_FOUND, which is ordinary and not a failure.
fn store_shared_floor(app: &AppHandle, entry_id: &str, states: &HashMap<i64, u32>) {
    let Some(webview) = app.get_webview(&webviews::webview_label(entry_id)) else {
        return;
    };

    // json object keys are strings, so the channel ids go over as strings and are read back by
    // `parse_shared_floor`
    let floor: HashMap<String, u32> = states
        .iter()
        .map(|(channel_id, count)| (channel_id.to_string(), *count))
        .collect();

    let Ok(payload) = serde_json::to_string(&floor) else {
        return;
    };

    let _ = webview.eval(format!(
        "window.__SHIVER_SET_READ_FLOOR__ && window.__SHIVER_SET_READ_FLOOR__({payload})"
    ));
}

/// Works out how much arrived on this server while Shiver was not watching it.
///
/// `read_states` is Sharkord's own per-channel unread — every message the user has never opened the
/// channel to read. Measured against the floor stored from the last time they opened this server,
/// the difference is what they have actually missed since then.
///
/// The first connection to a server has no floor, so it takes one and reports nothing. That is
/// deliberate: without it a freshly added public server would announce its entire history as
/// unread, which is true by Sharkord's reckoning and useless as a badge.
///
/// `shared` is the floor the companion plugin holds for this user on this server, and it takes
/// precedence over the local one: it is what the user's other devices are measuring from, and a
/// badge that disagrees between a phone and a desktop is the thing this exists to stop.
fn count_missed<R: Runtime>(
    app: &AppHandle<R>,
    entry_id: &str,
    read_states: &HashMap<i64, u32>,
    shared: Option<HashMap<i64, u32>>,
) {
    // A reconnect, not a first sight. Whatever is unread now is either already in the feed or
    // already counted here, and counting it again would show it twice. `ReadStates` gains this
    // entry immediately after the first join, which is what makes it the record of having looked.
    if app
        .state::<ReadStates>()
        .map()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(entry_id)
    {
        return;
    }

    let store = app.state::<Store>();

    let (local, muted) = {
        let registry = store.registry();

        let muted: Vec<i64> = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == entry_id)
            .map(|muted| muted.channel_id)
            .collect();

        (registry.baselines.get(entry_id).cloned(), muted)
    };

    // The shared floor wins wherever there is one. It is the same user's floor, kept by the
    // companion plugin against their account, so a server read on the phone is read here too —
    // which is the whole point of storing it there rather than on each device.
    let Some(baseline) = shared.or(local) else {
        // first sight of this server: establish the floor and say nothing about it
        let states = read_states.clone();
        let id = entry_id.to_string();

        let _ = store.update(|registry| {
            registry.baselines.insert(id.clone(), states.clone());

            Ok(())
        });

        return;
    };

    let missed = missed_since(read_states, &baseline, &muted);

    if missed > 0 {
        eprintln!("[shiver] {missed} message(s) arrived on {entry_id} while Shiver was away");
    }

    app.state::<Missed>().set(entry_id, missed);
}

/// How much of what a server reports unread is above the floor, ignoring muted channels.
///
/// Pure, so the arithmetic can be argued with directly — it is the part of this feature that is
/// easy to get quietly wrong, and a badge that is wrong by a few is not obviously a bug.
fn missed_since(
    read_states: &HashMap<i64, u32>,
    baseline: &HashMap<i64, u32>,
    muted: &[i64],
) -> usize {
    read_states
        .iter()
        // a muted channel is counted by the server and must not be counted here, the same rule the
        // feed applies to a live message
        .filter(|(channel_id, _)| !muted.contains(channel_id))
        .map(|(channel_id, count)| {
            // Saturating, because a channel can sit *below* its floor: the user read it elsewhere,
            // so the server's count went down. That subtracts nothing rather than wrapping to four
            // billion, which is what a plain `-` would do on a u32.
            count.saturating_sub(baseline.get(channel_id).copied().unwrap_or(0)) as usize
        })
        .sum()
}

/// Hands the server's conversation list to the inbox.
///
/// It arrives whole in the join payload, so unlike the page path — which learns of a conversation
/// when one turns up — a watched server's DM list is complete from the moment it connects.
fn publish_dms(app: &AppHandle, entry_id: &str, joined: &sharkord::Joined) {
    let (server_name, account_label) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let Some(entry) = registry.servers.iter().find(|server| server.id == entry_id) else {
            return;
        };

        let label = entry
            .account_label
            .clone()
            .or_else(|| entry.identity.clone())
            .unwrap_or_default();

        (entry.name.clone(), label)
    };

    let channels = joined
        .dms
        .iter()
        .map(|dm| DmChannel {
            channel_id: dm.channel_id,
            name: dm.user_name.clone(),
            icon_url: None,
            last_message_at: dm.last_message_at,
        })
        .collect();

    app.state::<Feed>()
        .set_dms(entry_id, &server_name, &account_label, channels);
}

/// Puts one message into the feed, which is where both the bell and the rail badge read it.
fn announce(
    app: &AppHandle,
    entry_id: &str,
    joined: &sharkord::Joined,
    message: &sharkord::NewMessage,
) {
    // a person's own message, arriving back down their own socket
    if message.user_id.is_some() && message.user_id == joined.own_user_id {
        return;
    }

    // This server has its own client running, and that client raises its own notifications. The
    // socket stays connected — it is the only thing that hears a read — but it does not also file
    // the messages, or every one would appear twice.
    if app
        .get_webview(&webviews::webview_label(entry_id))
        .is_some()
    {
        return;
    }

    let (server_name, muted) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let Some(entry) = registry.servers.iter().find(|server| server.id == entry_id) else {
            // a server removed while this connection was still open
            return;
        };

        let muted: Vec<i64> = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == entry_id)
            .map(|muted| muted.channel_id)
            .collect();

        (entry.name.clone(), muted)
    };

    let author = message
        .plugin_id
        .clone()
        .or_else(|| {
            message
                .user_id
                .and_then(|id| joined.user_names.get(&id).cloned())
        })
        // a name Shiver has never seen: somebody who joined after this connection did
        .unwrap_or_else(|| "Someone".into());

    let raw = RawNotification {
        channel_id: Some(message.channel_id),
        channel_name: joined.channel_names.get(&message.channel_id).cloned(),
        author,
        // an image or a file on its own, which is still worth being told about
        body: if message.text.is_empty() {
            "Sent an attachment".into()
        } else {
            message.text.clone()
        },
        icon_url: None,
        is_dm: joined.dm_channels.contains(&message.channel_id),
    };

    // Every string above came from a server. The bounds on them are applied inside `push`, which is
    // the one place every route into the feed passes through.
    let is_muted = muted.contains(&message.channel_id);

    if app
        .state::<Feed>()
        .push(entry_id, &server_name, raw, is_muted)
    {
        drain::notify_feed_changed(app);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::{missed_since, recount_without};

    type Counts = HashMap<String, usize>;
    type States = HashMap<String, HashMap<i64, u32>>;

    fn states(entry: &str, channels: &[(i64, u32)]) -> Mutex<States> {
        Mutex::new(HashMap::from([(
            entry.to_string(),
            channels.iter().copied().collect(),
        )]))
    }

    fn counts(entry: &str, count: usize) -> Mutex<Counts> {
        Mutex::new(HashMap::from([(entry.to_string(), count)]))
    }

    fn badge(counts: &Mutex<Counts>) -> Option<usize> {
        counts.lock().unwrap().get("entry-1").copied()
    }

    /* ── the counting ── */

    #[test]
    fn nothing_new_is_nothing_missed() {
        assert_eq!(
            missed_since(&HashMap::from([(1, 5)]), &HashMap::from([(1, 5)]), &[]),
            0
        );
    }

    #[test]
    fn only_what_arrived_above_the_floor_counts() {
        // the floor is why this is not just the server's own number: five were already there
        assert_eq!(
            missed_since(&HashMap::from([(1, 8)]), &HashMap::from([(1, 5)]), &[]),
            3
        );
    }

    #[test]
    fn a_channel_with_no_floor_counts_in_full() {
        assert_eq!(
            missed_since(&HashMap::from([(7, 4)]), &HashMap::from([(1, 5)]), &[]),
            4
        );
    }

    #[test]
    fn a_channel_read_elsewhere_subtracts_nothing() {
        // read on the phone, so the server now says less than the floor. A plain subtraction on a
        // u32 would wrap and badge the server with four billion unread.
        assert_eq!(
            missed_since(&HashMap::from([(1, 2)]), &HashMap::from([(1, 5)]), &[]),
            0
        );
    }

    #[test]
    fn muted_channels_are_not_counted() {
        assert_eq!(missed_since(&HashMap::from([(1, 9)]), &HashMap::new(), &[1]), 0);
        assert_eq!(
            missed_since(&HashMap::from([(1, 9), (2, 3)]), &HashMap::new(), &[1]),
            3
        );
    }

    /* ── reading a channel, which is the part that kept shipping broken ── */

    /// The reported bug, as a test.
    ///
    /// A direct message arrived, the user opened the server and read it — Sharkord's own unread
    /// marker cleared — and Shiver went on showing 1 on the server icon and 1 on the bell. **An
    /// open server has no socket**, because its page reports instead, so the page saying which
    /// channel it is showing is the only signal that anything was read at all.
    ///
    /// Four builds went out with some version of this broken. It is a test now.
    #[test]
    fn reading_the_channel_brings_the_badge_down() {
        let read_states = states("entry-1", &[(77, 1)]);
        let missed = counts("entry-1", 1);

        let moved = recount_without(
            &read_states,
            &missed,
            "entry-1",
            77,
            &HashMap::new(),
            &[],
        );

        assert!(moved, "reading the one unread channel is a change");
        assert_eq!(
            badge(&missed),
            None,
            "a server with nothing left unread is absent rather than zero"
        );
    }

    #[test]
    fn reading_one_channel_leaves_the_others_alone() {
        let read_states = states("entry-1", &[(77, 1), (88, 2)]);
        let missed = counts("entry-1", 3);

        assert!(recount_without(&read_states, &missed, "entry-1", 77, &HashMap::new(), &[]));
        assert_eq!(badge(&missed), Some(2), "the other channel's two are still unread");
    }

    #[test]
    fn a_channel_with_nothing_unread_changes_nothing() {
        let read_states = states("entry-1", &[(77, 1)]);
        let missed = counts("entry-1", 1);

        // opening a channel nobody had written in must not redraw anything
        assert!(!recount_without(&read_states, &missed, "entry-1", 999, &HashMap::new(), &[]));
        assert_eq!(badge(&missed), Some(1));
    }

    #[test]
    fn a_server_shiver_has_never_connected_to_is_left_alone() {
        let read_states: Mutex<States> = Mutex::new(HashMap::new());
        let missed: Mutex<Counts> = Mutex::new(HashMap::new());

        assert!(!recount_without(&read_states, &missed, "entry-1", 77, &HashMap::new(), &[]));
        assert!(missed.lock().unwrap().is_empty());
    }

    /// The floor still applies after a read: what was already there is still not news.
    #[test]
    fn the_floor_is_still_subtracted_after_a_read() {
        let read_states = states("entry-1", &[(77, 1), (88, 9)]);
        let missed = counts("entry-1", 1);
        let floor = HashMap::from([(88i64, 9u32)]);

        assert!(recount_without(&read_states, &missed, "entry-1", 77, &floor, &[]));
        assert_eq!(
            badge(&missed),
            None,
            "channel 88 sits on its floor, so only the read one was ever news"
        );
    }

    /// What arrived while Shiver was closed is counted **once**, not again on every reconnect.
    ///
    /// This is the property that makes the rail able to add the feed's count to this one. A socket
    /// that drops and comes back re-reads the server's unread, which still includes everything
    /// already sitting in the feed — so a second count here would badge each of those twice.
    #[test]
    fn a_reconnect_does_not_count_the_same_messages_again() {
        let floor = HashMap::from([(77i64, 1u32)]);
        let now = HashMap::from([(77i64, 4u32)]);

        // three arrived while Shiver was away
        assert_eq!(missed_since(&now, &floor, &[]), 3);

        // and while it is running, two more arrive — they go to the feed, and the rail adds the
        // two halves. What must not happen is this figure growing to five on the next reconnect,
        // which is what recomputing it against the server's current unread would do.
        let later = HashMap::from([(77i64, 6u32)]);

        assert_eq!(
            missed_since(&later, &floor, &[]),
            5,
            "the arithmetic itself is unchanged — `count_missed` is what must not run twice"
        );
    }

    /// A muted channel must not resurrect a badge when some other channel is read.
    #[test]
    fn muting_still_applies_after_a_read() {
        let read_states = states("entry-1", &[(77, 1), (88, 4)]);
        let missed = counts("entry-1", 5);

        assert!(recount_without(&read_states, &missed, "entry-1", 77, &HashMap::new(), &[88]));
        assert_eq!(badge(&missed), None, "the only unmuted unread was the one read");
    }
}
