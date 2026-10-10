//! The core's own connections to every server the webview is not showing (Android has one webview).
//!
//! Each connection keeps that server's unread count (above its floor) and its DM list, and hands
//! what arrives to `notify`. Sessions, passwords and floors live in the encrypted store
//! (`tauri-plugin-shiver-secrets`, see `sessions`); DM lists are `dms`.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

use shiver_core::{limit::Joins, LockExt};
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use crate::{
    sharkord,
    store::{RegistryStore, Store},
    webview,
};

mod dms;
mod sessions;

use dms::{now_ms, MAX_DMS};
use sessions::{baseline_store_key, forget_session, renew, store_off_thread};

pub use dms::{collect_dms, messages_seen, page_dms, DmEntry, PageDm};
pub use sessions::{
    forget_everywhere, forget_password, harvest_token, remember_password, remember_session,
    restore, session_for,
};

/// Emitted to Shiver's own pages with the per-server unread counts.
pub const INBOX_EVENT: &str = "shiver://inbox";

/// Refusals in a row, each after a fresh sign-in, before Shiver stops trying for a server.
const MAX_REFUSALS: u32 = 3;

/// How long each server left goes unwatched: switching away and back reloads its page, and
/// opening and closing a connection of Shiver's own besides doubled what the server saw.
pub(crate) const WATCH_GRACE: Duration = Duration::from_secs(30);

const MUTE_POLL: Duration = Duration::from_secs(1);

#[derive(Default)]
struct State {
    /// wiped when replaced or dropped; copies handed out are short-lived
    tokens: HashMap<String, Zeroizing<String>>,
    unread: HashMap<String, u32>,
    running: HashMap<String, tauri::async_runtime::JoinHandle<()>>,
    /// the one pending `sync` for when the next server's `WATCH_GRACE` runs out
    grace_timer: Option<tauri::async_runtime::JoinHandle<()>>,
    /// each entry's conversations, from its last join or from its page while on screen
    dms: HashMap<String, Vec<sharkord::DirectMessage>>,
    /// the latest message time known per entry and channel, from whichever source told it first
    dm_times: HashMap<String, HashMap<i64, u64>>,
    baselines: HashMap<String, HashMap<i64, u32>>,
    read_states: HashMap<String, HashMap<i64, u32>>,
    signed_out: HashSet<String>,
    /// entries with a stored password
    remembered: HashSet<String>,
    problems: HashMap<String, String>,
    plugins: HashMap<String, Option<String>>,
}

#[derive(Default)]
pub struct Inbox(Mutex<State>);

impl Inbox {
    fn with<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        change(&mut self.0.locked())
    }

    /// Records a token; returns whether it was new.
    pub fn remember_token(&self, entry_id: &str, token: &str) -> bool {
        self.with(|state| {
            state
                .tokens
                .insert(entry_id.to_string(), Zeroizing::new(token.to_string()))
                .is_none_or(|previous| previous.as_str() != token)
        })
    }

    pub fn token(&self, entry_id: &str) -> Option<String> {
        self.with(|state| state.tokens.get(entry_id).map(|token| token.to_string()))
    }

    /// Drops the session and everything derived from it, stopping the connection.
    fn forget_token(&self, entry_id: &str) {
        self.with(|state| {
            state.tokens.remove(entry_id);
            state.unread.remove(entry_id);
            state.read_states.remove(entry_id);

            if let Some(task) = state.running.remove(entry_id) {
                task.abort();
            }
        });
    }

    /// Drops everything about an entry.
    fn forget(&self, entry_id: &str) {
        self.forget_token(entry_id);
        self.with(|state| {
            state.signed_out.remove(entry_id);
            state.remembered.remove(entry_id);
            state.dms.remove(entry_id);
            state.dm_times.remove(entry_id);
            state.baselines.remove(entry_id);
            state.problems.remove(entry_id);
            state.plugins.remove(entry_id);
        });
    }

    /// Records a problem; returns whether it is the first for that entry.
    pub(crate) fn remember_problem(&self, entry_id: &str, reason: &str) -> bool {
        self.with(|state| {
            state
                .problems
                .insert(entry_id.to_string(), reason.to_string())
                .is_none()
        })
    }

    pub fn problems(&self) -> HashMap<String, String> {
        self.with(|state| state.problems.clone())
    }

    pub fn plugins(&self) -> HashMap<String, Option<String>> {
        self.with(|state| state.plugins.clone())
    }

    pub fn dms(&self) -> HashMap<String, Vec<sharkord::DirectMessage>> {
        self.with(|state| state.dms.clone())
    }

    pub fn dm_times(&self) -> HashMap<String, HashMap<i64, u64>> {
        self.with(|state| state.dm_times.clone())
    }

    pub fn unread(&self) -> HashMap<String, u32> {
        self.with(|state| state.unread.clone())
    }

    pub fn signed_out(&self) -> Vec<String> {
        self.with(|state| state.signed_out.iter().cloned().collect())
    }

    pub fn baseline(&self, entry_id: &str) -> Option<HashMap<i64, u32>> {
        self.with(|state| state.baselines.get(entry_id).cloned())
    }

    pub fn has_password(&self, entry_id: &str) -> bool {
        self.with(|state| state.remembered.contains(entry_id))
    }

    /// A message was sent or received in one of an entry's conversations at `at` (never past now),
    /// which moves it up the list; kept even before the conversation itself is known, since a page
    /// can report the time before its list. Returns whether the time moved.
    fn dm_active(&self, entry_id: &str, channel_id: i64, at: u64) -> bool {
        let at = at.min(now_ms());

        self.with(|state| {
            let times = state.dm_times.entry(entry_id.to_string()).or_default();

            if times.get(&channel_id).is_some_and(|known| *known >= at)
                || (times.len() >= MAX_DMS && !times.contains_key(&channel_id))
            {
                return false;
            }

            times.insert(channel_id, at);

            true
        })
    }

    fn set_unread(&self, entry_id: &str, count: u32) -> bool {
        self.with(|state| state.unread.insert(entry_id.to_string(), count) != Some(count))
    }
}

/* ── the encrypted store ── */

/// Starts a connection for every entry with a session, no connection and no problem that retrying
/// cannot fix, stops those for entries that left the rail or are on screen (their page has its
/// own, until Shiver has been in the background for `WATCH_GRACE`), and settles the notification
/// of the server on screen.
pub fn sync(app: &AppHandle) {
    let registry_ids: HashSet<String> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .map(|server| server.id.clone())
        .collect();
    let showing = app.state::<webview::Showing>().kept_by_page(WATCH_GRACE);
    let just_left = app.state::<webview::Showing>().just_left(WATCH_GRACE);
    let watchable = |id: &String| {
        registry_ids.contains(id) && showing.as_ref() != Some(id) && !just_left.contains_key(id)
    };

    let unwanted = app.state::<Inbox>().with(|state| {
        let unwanted: Vec<String> = state
            .running
            .keys()
            .filter(|id| !watchable(id))
            .cloned()
            .collect();
        let wanted: Vec<String> = state
            .tokens
            .keys()
            .filter(|id| {
                watchable(id)
                    && !state.running.contains_key(*id)
                    && !state.problems.contains_key(*id)
            })
            .cloned()
            .collect();

        for entry_id in &unwanted {
            if let Some(task) = state.running.remove(entry_id) {
                task.abort();
            }

            state.unread.remove(entry_id);
        }

        // spawned and recorded under the same lock that saw them missing, so two syncs at once
        // cannot both start one (spawning does not block, and the task waits for the lock)
        for entry_id in wanted {
            let task = tauri::async_runtime::spawn(watch(app.clone(), entry_id.clone()));

            if let Some(earlier) = state.running.insert(entry_id, task) {
                earlier.abort();
            }
        }

        unwanted
    });

    for entry_id in unwanted.iter().chain(showing.iter()) {
        crate::notify::clear(app, entry_id);
    }

    if let Some(rest) = just_left.into_values().min() {
        let handle = app.clone();
        let timer = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(rest).await;
            sync(&handle);
        });

        // replaced rather than added to: sync runs often, and each would otherwise leave a timer
        if let Some(earlier) = app
            .state::<Inbox>()
            .with(|state| state.grace_timer.replace(timer))
        {
            earlier.abort();
        }
    }

    publish(app);
}

/// Shiver went to the background (`onPause`) or came back (`onResume`). A server left on screen is
/// watched by the core after `WATCH_GRACE` in the background, so its messages still notify; coming
/// back hands it to its page again.
#[cfg_attr(not(mobile), allow(dead_code))]
pub fn set_background(app: &AppHandle, background: bool) {
    app.state::<webview::Showing>().set_background(background);

    if !background {
        return sync(app);
    }

    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(WATCH_GRACE).await;

        if app.state::<webview::Showing>().in_background() {
            sync(&app);
        }
    });
}

/// Drops a server's connection and any problem with it, so the next attempt uses what changed.
pub fn restart(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>().with(|state| {
        state.problems.remove(entry_id);

        if let Some(task) = state.running.remove(entry_id) {
            task.abort();
        }
    });

    sync(app);
}

/// Watches one server with the session Shiver holds. A refused session is renewed from the stored
/// password; repeated refusals give up and mark the server signed out.
async fn watch(app: AppHandle, entry_id: String) {
    let key = entry_id.clone();

    sharkord::watch(&key, Watch { app, entry_id }).await;
}

struct Watch {
    app: AppHandle,
    entry_id: String,
}

impl sharkord::Watcher for Watch {
    async fn target(&mut self) -> Option<sharkord::Target> {
        // asked each attempt, which is what renews an expiring session
        let token = session_for(&self.app, &self.entry_id).await;
        let server = self
            .app
            .state::<Store>()
            .registry()
            .server(&self.entry_id)
            .map(|server| (server.origin.clone(), server.accept_any_size));

        let (Some(token), Some((origin, accept_any_size))) = (token, server) else {
            self.app
                .state::<Inbox>()
                .with(|state| state.running.remove(&self.entry_id));

            return None;
        };

        // each connection joins the server, which Sharkord allows only a few times a minute
        let place = self.app.state::<Joins>().take(&origin);

        tokio::time::sleep(place.wait).await;

        Some(sharkord::Target {
            origin,
            token: Zeroizing::new(token),
            accept_any_size,
        })
    }

    fn joined(&mut self, joined: &sharkord::Joined) {
        on_joined(&self.app, &self.entry_id, joined);
    }

    fn event(&mut self, joined: &sharkord::Joined, event: sharkord::Event) {
        let (app, entry_id) = (&self.app, self.entry_id.as_str());

        match event {
            sharkord::Event::Unread { channel_id, delta } => {
                update_read_states(app, entry_id, |states| {
                    sharkord::apply_delta(states, channel_id, delta)
                })
            }
            sharkord::Event::UnreadSet { channel_id, count } => {
                update_read_states(app, entry_id, |states| {
                    sharkord::set_unread(states, channel_id, count)
                })
            }
            sharkord::Event::Posted(message) => {
                // the user's own messages count too: the list is ordered by the latest in each
                if joined.dm_channels.contains(&message.channel_id)
                    && app
                        .state::<Inbox>()
                        .dm_active(entry_id, message.channel_id, now_ms())
                {
                    webview::emit_home(app, INBOX_EVENT, app.state::<Inbox>().unread());
                }

                crate::notify::announce(app, entry_id, joined, &message)
            }
        }
    }

    async fn refused(&mut self, refusals: u32) -> bool {
        if refusals < MAX_REFUSALS && renew(&self.app, &self.entry_id).await.is_some() {
            return true;
        }

        forget_session(&self.app, &self.entry_id);

        false
    }

    fn too_large(&mut self, size: usize) {
        crate::notify::report_watch_problem(&self.app, &self.entry_id, size);
        self.app
            .state::<Inbox>()
            .with(|state| state.running.remove(&self.entry_id));
    }
}

/// Takes the floor (the plugin's shared one wins; else the first join's counts), then the DMs,
/// plugin version and read states.
fn on_joined(app: &AppHandle, entry_id: &str, joined: &sharkord::Joined) {
    let floor = app.state::<Inbox>().with(|state| {
        state.problems.remove(entry_id);
        state.dms.insert(entry_id.to_string(), joined.dms.clone());
        state
            .plugins
            .insert(entry_id.to_string(), joined.plugin_version.clone());

        let (floor, changed) = sharkord::choose_floor(joined, state.baselines.get(entry_id));

        state.baselines.insert(entry_id.to_string(), floor.clone());
        changed.then_some(floor)
    });

    if let Some(floor) = floor {
        if let Ok(encoded) = serde_json::to_string(&floor) {
            store_off_thread(
                app,
                baseline_store_key(entry_id),
                Some(Zeroizing::new(encoded)),
            );
        }
    }

    update_read_states(app, entry_id, |states| *states = joined.read_states.clone());
}

/// Changes one entry's read states in place and recomputes its unread count.
fn update_read_states(
    app: &AppHandle,
    entry_id: &str,
    change: impl FnOnce(&mut HashMap<i64, u32>),
) {
    let muted: HashSet<i64> = app
        .state::<Store>()
        .registry()
        .muted_for(entry_id)
        .into_iter()
        .collect();

    let total = app.state::<Inbox>().with(|state| {
        let states = state.read_states.entry(entry_id.to_string()).or_default();

        change(states);

        let empty = HashMap::new();

        sharkord::unread_total(
            states,
            state.baselines.get(entry_id).unwrap_or(&empty),
            &muted,
        )
    });

    if app.state::<Inbox>().set_unread(entry_id, total) {
        publish(app);
    }
}

/// Replaces an entry's mutes with what its page reports (bounded), and recounts.
pub fn replace_mutes(app: &AppHandle, entry_id: &str, channels: &[i64]) {
    let store = app.state::<Store>();
    let next = shiver_core::model::normalized_mutes(channels.iter().copied());

    if store.registry().muted_for(entry_id) == next {
        return;
    }

    if let Err(error) = store.update(|registry| Ok(registry.set_muted_for(entry_id, next))) {
        return eprintln!("[shiver] could not store {entry_id}'s muted channels: {error}");
    }

    update_read_states(app, entry_id, |_| ());
}

/// Polls the page on screen for its mutes, rail changes and link openings.
pub fn watch_mutes(app: &AppHandle) {
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(MUTE_POLL);

        loop {
            ticker.tick().await;

            let showing = handle.state::<webview::Showing>();

            // nobody changes a mute in the background, and the poll would keep the page busy
            if showing.in_background() {
                continue;
            }

            if let Some(entry_id) = showing.server() {
                webview::read_mutes(&handle, &entry_id);
            }
        }
    });
}

/// Sends the unread counts to Shiver's own page.
fn publish(app: &AppHandle) {
    webview::emit_home(app, INBOX_EVENT, app.state::<Inbox>().unread());
    crate::rail::refresh(app);
}

/* ── sessions ── */

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(channel_id: i64, name: &str, at: Option<u64>) -> sharkord::DirectMessage {
        sharkord::DirectMessage {
            channel_id,
            user_name: name.into(),
            last_message_at: at,
            avatar_url: None,
        }
    }

    #[test]
    fn a_conversation_moves_up_only_and_never_past_now() {
        let inbox = Inbox::default();

        inbox.with(|state| {
            state
                .dms
                .insert("a".into(), vec![conversation(3, "Robin", Some(5_000))])
        });

        let time = || inbox.dm_times()["a"].get(&3).copied();

        assert!(inbox.dm_active("a", 3, 5_000));
        assert!(!inbox.dm_active("a", 3, 4_000));
        assert_eq!(time(), Some(5_000));
        assert!(inbox.dm_active("a", 3, 6_000));
        assert_eq!(time(), Some(6_000));
        assert!(inbox.dm_active("a", 3, u64::MAX));
        assert!(time() <= Some(now_ms()));
        // a time can come before its conversation does; collect_dms then puts the two together
        assert!(inbox.dm_active("b", 3, 6_000));
    }

    #[test]
    fn tokens_and_counts_report_only_changes_and_forget_clears_them() {
        let inbox = Inbox::default();

        assert!(inbox.remember_token("a", "one"));
        assert!(!inbox.remember_token("a", "one"));
        assert!(inbox.set_unread("a", 3));
        assert!(!inbox.set_unread("a", 3));

        inbox.forget("a");

        assert!(inbox.unread().is_empty());
        assert!(inbox.token("a").is_none());
    }
}
