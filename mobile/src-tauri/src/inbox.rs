//! The core's own connections to every server the webview is not showing (Android has one webview).
//!
//! Each connection keeps that server's unread count (above its floor), its DM list, and posts one
//! Android notification per server that summarises what arrived. Sessions, passwords and floors live
//! in the encrypted store (`tauri-plugin-shiver-secrets`), written by one
//! background thread in order.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

use shiver_core::{text::clamp, LockExt};
use tauri::{AppHandle, Manager};
use tauri_plugin_shiver_secrets::SecretsExt;

use crate::{
    sharkord,
    store::{RegistryStore, Store},
    webview,
};

/// Emitted to Shiver's own pages with the per-server unread counts.
pub const INBOX_EVENT: &str = "shiver://inbox";

const MAX_AUTHOR: usize = 100;
const MAX_BODY: usize = 500;
const MAX_CHANNEL_NAME: usize = 100;

/// Refusals in a row, each after a fresh sign-in, before Shiver stops trying for a server.
const MAX_REFUSALS: u32 = 3;

/// Messages arriving this close together are posted as one notification.
const NOTIFY_AFTER: Duration = Duration::from_millis(900);

const MUTE_POLL: Duration = Duration::from_secs(1);

const BASELINE_PREFIX: &str = "baseline:";
const PASSWORD_PREFIX: &str = "password:";

/// The longest session token kept; the page decides what it stores.
const MAX_TOKEN: usize = 8 * 1024;

/// Reads a page's session: Sharkord's persistent auto-login token (`kept:`, which the user asked to
/// keep, so Shiver stores it) or the live session only (`live:`, used for this run only). A bare
/// expression, because `eval_with_callback` drops the value of a function with a `try` in it.
const TOKEN_SCRIPT: &str = "localStorage.getItem('sharkord-auto-login-token') \
     ? 'kept:' + localStorage.getItem('sharkord-auto-login-token') \
     : (sessionStorage.getItem('sharkord-token') \
        ? 'live:' + sessionStorage.getItem('sharkord-token') : null)";

/// One conversation and the rail entry (account) it belongs to. Only Shiver's own pages see these.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmEntry {
    pub entry_id: String,
    pub server_name: String,
    pub account_label: Option<String>,
    pub channel_id: i64,
    pub user_name: String,
    pub last_message_at: Option<u64>,
}

/// Every server's conversations, newest first (ties by name), independent of rail order.
pub fn collect_dms(
    servers: &[crate::model::ServerEntry],
    dms: &HashMap<String, Vec<sharkord::DirectMessage>>,
) -> Vec<DmEntry> {
    let mut collected: Vec<DmEntry> = servers
        .iter()
        .flat_map(|server| {
            dms.get(&server.id)
                .into_iter()
                .flatten()
                .map(move |dm| DmEntry {
                    entry_id: server.id.clone(),
                    server_name: server.name.clone(),
                    account_label: server.account_label.clone(),
                    channel_id: dm.channel_id,
                    user_name: dm.user_name.clone(),
                    last_message_at: dm.last_message_at,
                })
        })
        .collect();

    collected.sort_by(|a, b| {
        b.last_message_at
            .cmp(&a.last_message_at)
            .then_with(|| a.user_name.cmp(&b.user_name))
    });
    collected
}

#[derive(Debug, Clone, Default)]
struct Announcement {
    count: u32,
    latest: String,
}

#[derive(Default)]
struct State {
    tokens: HashMap<String, String>,
    unread: HashMap<String, u32>,
    running: HashMap<String, tauri::async_runtime::JoinHandle<()>>,
    announced: HashMap<String, Announcement>,
    /// entries with a notification post scheduled
    posting: HashSet<String>,
    dms: HashMap<String, Vec<sharkord::DirectMessage>>,
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
                .insert(entry_id.to_string(), token.to_string())
                .as_deref()
                != Some(token)
        })
    }

    pub fn token(&self, entry_id: &str) -> Option<String> {
        self.with(|state| state.tokens.get(entry_id).cloned())
    }

    /// Drops the session and everything derived from it, stopping the connection.
    fn forget_token(&self, entry_id: &str) {
        self.with(|state| {
            state.tokens.remove(entry_id);
            state.unread.remove(entry_id);
            state.announced.remove(entry_id);
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
            state.baselines.remove(entry_id);
            state.problems.remove(entry_id);
            state.plugins.remove(entry_id);
        });
    }

    /// Records a problem; returns whether it is the first for that entry.
    fn remember_problem(&self, entry_id: &str, reason: &str) -> bool {
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

    fn set_unread(&self, entry_id: &str, count: u32) -> bool {
        self.with(|state| state.unread.insert(entry_id.to_string(), count) != Some(count))
    }
}

/* ── the encrypted store ── */

fn baseline_store_key(entry_id: &str) -> String {
    format!("{BASELINE_PREFIX}{entry_id}")
}

fn password_store_key(entry_id: &str) -> String {
    format!("{PASSWORD_PREFIX}{entry_id}")
}

/// The entry a store key belongs to (a bare key is the entry's session).
fn entry_of(key: &str) -> &str {
    [PASSWORD_PREFIX, BASELINE_PREFIX]
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .unwrap_or(key)
}

/// Queues a write (`Some`) or removal (`None`) for the one store-writer thread, which keeps writes
/// in order and off the UI thread (each is a blocking call into the JVM).
fn store_off_thread(app: &AppHandle, key: String, value: Option<String>) {
    static WRITER: std::sync::OnceLock<std::sync::mpsc::Sender<(String, Option<String>)>> =
        std::sync::OnceLock::new();

    let sender = WRITER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<(String, Option<String>)>();
        let handle = app.clone();

        std::thread::Builder::new()
            .name("shiver-store".into())
            .spawn(move || {
                for (key, value) in receiver {
                    let result = match value {
                        Some(value) => handle.shiver_secrets().set(&key, &value),
                        None => handle.shiver_secrets().remove(&key),
                    };

                    if let Err(error) = result {
                        eprintln!(
                            "[shiver] could not update the encrypted store for {}: {error}",
                            entry_of(&key)
                        );
                    }
                }
            })
            .expect("the store writer thread should start");

        sender
    });

    let _ = sender.send((key, value));
}

/// Loads sessions, passwords and floors at startup (off the main thread), drops
/// anything for entries no longer in the rail, then connects.
pub fn restore(app: &AppHandle) {
    let app = app.clone();

    std::thread::spawn(move || {
        let keys = match app.shiver_secrets().keys() {
            Ok(keys) => keys,
            Err(error) => return eprintln!("[shiver] could not read the encrypted store: {error}"),
        };

        let known: HashSet<String> = app
            .state::<Store>()
            .registry()
            .servers
            .iter()
            .map(|server| server.id.clone())
            .collect();
        let inbox = app.state::<Inbox>();

        for key in keys {
            let entry_id = entry_of(&key).to_string();

            if !known.contains(&entry_id) {
                let _ = app.shiver_secrets().remove(&key);

                continue;
            }

            let Ok(Some(value)) = app.shiver_secrets().get(&key) else {
                continue;
            };

            if key.starts_with(BASELINE_PREFIX) {
                match serde_json::from_str::<HashMap<i64, u32>>(&value) {
                    Ok(baseline) => inbox.with(|state| state.baselines.insert(entry_id, baseline)),
                    Err(_) => {
                        let _ = app.shiver_secrets().remove(&key);

                        None
                    }
                };
            } else if key.starts_with(PASSWORD_PREFIX) {
                inbox.with(|state| state.remembered.insert(entry_id));
            } else {
                inbox.remember_token(&entry_id, &value);
            }
        }

        sync(&app);
        sign_in_missing(&app);
    });
}

/// Signs in the servers that have a stored password but no session.
fn sign_in_missing(app: &AppHandle) {
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let missing: Vec<(String, String)> = {
            let inbox = handle.state::<Inbox>();
            let store = handle.state::<Store>();
            let registry = store.registry();

            registry
                .servers
                .iter()
                .filter(|server| server.identity.is_some())
                .filter(|server| {
                    inbox.token(&server.id).is_none() && inbox.has_password(&server.id)
                })
                .map(|server| (server.id.clone(), server.origin.clone()))
                .collect()
        };

        for (entry_id, origin) in missing {
            sign_in_again(&handle, &entry_id, &origin).await;
        }
    });
}

/// Keeps a session (in memory, and in the store when `persist`) and connects with it. A token past
/// `MAX_TOKEN` is not kept.
fn record_session(app: &AppHandle, entry_id: &str, token: &str, persist: bool) {
    if token.len() > MAX_TOKEN {
        return;
    }

    if app
        .state::<Inbox>()
        .with(|state| state.signed_out.remove(entry_id))
    {
        publish(app);
    }

    if !app.state::<Inbox>().remember_token(entry_id, token) {
        return;
    }

    if persist {
        store_off_thread(app, entry_id.to_string(), Some(token.to_string()));
    }

    sync(app);
}

pub fn remember_session(app: &AppHandle, entry_id: &str, token: &str) {
    record_session(app, entry_id, token, true);
}

pub fn remember_password(app: &AppHandle, entry_id: &str, password: &str) {
    app.state::<Inbox>().with(|state| {
        state.remembered.insert(entry_id.to_string());
        state.signed_out.remove(entry_id);
    });
    store_off_thread(
        app,
        password_store_key(entry_id),
        Some(password.to_string()),
    );
}

pub fn forget_password(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>()
        .with(|state| state.remembered.remove(entry_id));
    store_off_thread(app, password_store_key(entry_id), None);
}

/// Drops the session (not the password) and marks the server as waiting for a sign-in.
fn forget_session(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>().forget_token(entry_id);
    store_off_thread(app, entry_id.to_string(), None);

    if app
        .state::<Inbox>()
        .with(|state| state.signed_out.insert(entry_id.to_string()))
    {
        publish(app);
    }
}

/// Drops everything Shiver holds for an entry, in memory and in the store.
pub fn forget_everywhere(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>().forget(entry_id);
    clear_notification(app, entry_id);

    for key in [
        entry_id.to_string(),
        password_store_key(entry_id),
        baseline_store_key(entry_id),
    ] {
        store_off_thread(app, key, None);
    }
}

/* ── connections ── */

/// Starts a connection for every entry with a session, no connection and no problem that retrying
/// cannot fix, stops those for entries that left the rail or are on screen (their page has its
/// own), and settles the notification of the server on screen.
pub fn sync(app: &AppHandle) {
    let registry_ids: HashSet<String> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .map(|server| server.id.clone())
        .collect();
    let showing = app.state::<webview::Showing>().server();
    let watchable = |id: &String| registry_ids.contains(id) && showing.as_ref() != Some(id);

    let (wanted, unwanted) = app.state::<Inbox>().with(|state| {
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

        (wanted, unwanted)
    });

    for entry_id in unwanted.iter().chain(showing.iter()) {
        clear_notification(app, entry_id);
    }

    for entry_id in wanted {
        let task = tauri::async_runtime::spawn(watch(app.clone(), entry_id.clone()));

        app.state::<Inbox>()
            .with(|state| state.running.insert(entry_id, task));
    }

    publish(app);
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
        let token = self.app.state::<Inbox>().token(&self.entry_id);
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

        Some(sharkord::Target {
            origin,
            token,
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
            sharkord::Event::Posted(message) => announce(app, entry_id, joined, &message),
        }
    }

    async fn refused(&mut self, refusals: u32) -> bool {
        let origin = self
            .app
            .state::<Store>()
            .registry()
            .server(&self.entry_id)
            .map(|server| server.origin.clone());

        if let Some(origin) = origin {
            if refusals < MAX_REFUSALS && sign_in_again(&self.app, &self.entry_id, &origin).await {
                return true;
            }
        }

        forget_session(&self.app, &self.entry_id);

        false
    }

    fn too_large(&mut self, size: usize) {
        report_watch_problem(&self.app, &self.entry_id, size);
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

        let floor = joined
            .shared_floor
            .clone()
            .or_else(|| state.baselines.get(entry_id).cloned())
            .unwrap_or_else(|| joined.read_states.clone());
        let changed = state.baselines.get(entry_id) != Some(&floor);

        state.baselines.insert(entry_id.to_string(), floor.clone());
        changed.then_some(floor)
    });

    if let Some(floor) = floor {
        if let Ok(encoded) = serde_json::to_string(&floor) {
            store_off_thread(app, baseline_store_key(entry_id), Some(encoded));
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

            if let Some(entry_id) = handle.state::<webview::Showing>().server() {
                webview::read_mutes(&handle, &entry_id);
            }
        }
    });
}

/// Sends the unread counts to Shiver's own page.
fn publish(app: &AppHandle) {
    webview::emit_home(app, INBOX_EVENT, app.state::<Inbox>().unread());
}

/// Signs a server in again from its stored password. A refused password is forgotten.
async fn sign_in_again(app: &AppHandle, entry_id: &str, origin: &str) -> bool {
    let Some(identity) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .and_then(|server| server.identity.clone())
    else {
        return false;
    };

    let handle = app.clone();
    let key = password_store_key(entry_id);
    let password =
        match tokio::task::spawn_blocking(move || handle.shiver_secrets().get(&key)).await {
            Ok(Ok(Some(password))) => zeroize::Zeroizing::new(password),
            Ok(Ok(None)) => return false,
            Ok(Err(error)) => {
                eprintln!("[shiver] could not read the stored password for {origin}: {error}");

                return false;
            }
            Err(_) => return false,
        };

    match shiver_core::login::sign_in(origin, &identity, &password).await {
        Ok(session) => {
            remember_session(app, entry_id, &session);

            true
        }
        Err(error @ shiver_core::Error::Refused(_)) => {
            eprintln!("[shiver] {origin} refused Shiver's stored password: {error}");
            forget_password(app, entry_id);

            false
        }
        Err(error) => {
            eprintln!("[shiver] could not reach {origin} to sign in again: {error}");

            false
        }
    }
}

/// After a server page loads, reads its session so the core can watch it once the user moves on.
pub fn harvest_token(app: &AppHandle, entry_id: String) {
    let Ok(window) = webview::main_window(app) else {
        return;
    };

    let handle = app.clone();

    let _ = window.eval_with_callback(TOKEN_SCRIPT, move |raw| {
        let Ok(serde_json::Value::String(answer)) = serde_json::from_str::<serde_json::Value>(&raw)
        else {
            return;
        };

        let Some((token, persist)) = answer
            .strip_prefix("kept:")
            .map(|token| (token.to_string(), true))
            .or_else(|| {
                answer
                    .strip_prefix("live:")
                    .map(|token| (token.to_string(), false))
            })
            .filter(|(token, _)| !token.is_empty())
        else {
            return;
        };

        // off this thread: storing calls into the JVM, which deadlocks from an eval callback
        let (handle, id) = (handle.clone(), entry_id.clone());

        std::thread::spawn(move || record_session(&handle, &id, &token, persist));
    });
}

/* ── notifications ── */

/// One notification id per server, shared with `PushReceiver` (which uses Java's
/// `String.hashCode` of the push token), so either side can replace or clear the other's.
fn notification_id(app: &AppHandle, entry_id: &str) -> i32 {
    let token = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .and_then(|server| server.push_token.clone())
        .unwrap_or_else(|| entry_id.to_string());

    shiver_core::hash::java_string(&token)
}

fn announce(
    app: &AppHandle,
    entry_id: &str,
    joined: &sharkord::Joined,
    message: &sharkord::NewMessage,
) {
    if app.state::<webview::Showing>().server().as_deref() == Some(entry_id) {
        return;
    }

    let (muted, server) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        (
            registry.muted_for(entry_id),
            registry.server(entry_id).map(|server| server.name.clone()),
        )
    };

    let (Some(server), Some(line)) = (server, notice(joined, &muted, message)) else {
        return;
    };

    app.state::<Inbox>().with(|state| {
        let announcement = state.announced.entry(entry_id.to_string()).or_default();

        announcement.count += 1;
        announcement.latest = line;
    });

    // one post per burst: the first message schedules it, later ones update what it will say
    if !app
        .state::<Inbox>()
        .with(|state| state.posting.insert(entry_id.to_string()))
    {
        return;
    }

    let (app, entry_id) = (app.clone(), entry_id.to_string());

    std::thread::spawn(move || {
        std::thread::sleep(NOTIFY_AFTER);

        let announcement = app.state::<Inbox>().with(|state| {
            state.posting.remove(&entry_id);
            state.announced.get(&entry_id).cloned()
        });

        if let Some(announcement) = announcement {
            post(
                &app,
                notification_id(&app, &entry_id),
                server,
                &announcement,
            );
        }
    });
}

/// The notification line for a message, or `None` for the user's own or a muted channel's.
fn notice(
    joined: &sharkord::Joined,
    muted: &[i64],
    message: &sharkord::NewMessage,
) -> Option<String> {
    if message.is_own(joined) || muted.contains(&message.channel_id) {
        return None;
    }

    let author = clamp(message.author(joined), MAX_AUTHOR);
    let text = clamp(message.body().to_string(), MAX_BODY);

    if joined.dm_channels.contains(&message.channel_id) {
        return Some(format!("{author}: {text}"));
    }

    let channel = clamp(
        joined
            .channel_names
            .get(&message.channel_id)
            .cloned()
            .unwrap_or_else(|| "a channel".into()),
        MAX_CHANNEL_NAME,
    );

    Some(format!("{author} in #{channel}: {text}"))
}

fn post(app: &AppHandle, id: i32, title: String, announcement: &Announcement) {
    use tauri_plugin_notification::NotificationExt;

    let body = match announcement.count {
        0 | 1 => announcement.latest.clone(),
        count => format!("{}\n… and {} more", announcement.latest, count - 1),
    };

    if let Err(error) = app
        .notification()
        .builder()
        .id(id)
        .title(title)
        .body(body)
        .show()
    {
        eprintln!("[shiver] could not show a notification: {error}");
    }
}

/// Removes an entry's notification, if one was posted.
fn clear_notification(app: &AppHandle, entry_id: &str) {
    let announced = app.state::<Inbox>().with(|state| {
        state.posting.remove(entry_id);
        state.announced.remove(entry_id).is_some()
    });

    #[cfg(mobile)]
    if announced {
        use tauri_plugin_notification::NotificationExt;

        let id = notification_id(app, entry_id);
        let app = app.clone();

        std::thread::spawn(move || {
            let _ = app.notification().remove_active(vec![id]);
        });
    }

    #[cfg(not(mobile))]
    let _ = announced;
}

/// Says once, with a notification, that a server sends more than Shiver accepts.
fn report_watch_problem(app: &AppHandle, entry_id: &str, size: usize) {
    let Some(name) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .map(|server| server.name.clone())
    else {
        return;
    };

    let size = sharkord::readable_size(size);

    let first = app.state::<Inbox>().remember_problem(
        entry_id,
        &format!("Sends {size} in one message, which is more than Shiver accepts — so nothing from this server reaches this phone."),
    );

    #[cfg(target_os = "android")]
    if first {
        use tauri_plugin_notification::NotificationExt;

        let _ = app
            .notification()
            .builder()
            .id(shiver_core::hash::java_string(&format!("problem:{entry_id}")))
            .title(format!("Shiver cannot watch {name}"))
            .body("This server sends more in one message than Shiver accepts, so its messages will not reach you here. Settings has the details.")
            .show();
    }

    #[cfg(not(target_os = "android"))]
    let _ = (first, name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> sharkord::Joined {
        sharkord::Joined {
            own_user_id: Some(1),
            dm_channels: vec![9],
            channel_names: HashMap::from([(4, "general".to_string())]),
            user_names: HashMap::from([(2, "Smiddy".to_string())]),
            ..Default::default()
        }
    }

    fn from(user_id: i64, channel_id: i64, text: &str) -> sharkord::NewMessage {
        sharkord::NewMessage {
            channel_id,
            user_id: Some(user_id),
            plugin_id: None,
            text: text.to_string(),
        }
    }

    fn entry(id: &str, position: i32) -> crate::model::ServerEntry {
        crate::model::ServerEntry {
            id: id.into(),
            origin: format!("https://{id}.example.com"),
            name: id.to_uppercase(),
            position,
            ..Default::default()
        }
    }

    fn conversation(channel_id: i64, name: &str, at: Option<u64>) -> sharkord::DirectMessage {
        sharkord::DirectMessage {
            channel_id,
            user_name: name.into(),
            last_message_at: at,
        }
    }

    #[test]
    fn a_message_reads_as_who_said_what_and_where() {
        assert_eq!(
            notice(&server(), &[], &from(2, 4, "hi?")),
            Some("Smiddy in #general: hi?".into())
        );
        assert_eq!(
            notice(&server(), &[], &from(2, 9, "hello")),
            Some("Smiddy: hello".into())
        );
        assert_eq!(
            notice(&server(), &[], &from(2, 4, "")),
            Some("Smiddy in #general: Sent an attachment".into())
        );
        assert_eq!(
            notice(&server(), &[], &from(77, 88, "hi")),
            Some("Someone in #a channel: hi".into())
        );
        assert_eq!(notice(&server(), &[], &from(1, 4, "mine")), None);
        assert_eq!(notice(&server(), &[4], &from(2, 4, "muted")), None);
    }

    #[test]
    fn long_fields_are_shortened_by_characters() {
        let long_name = sharkord::NewMessage {
            plugin_id: Some("é".repeat(5_000)),
            user_id: None,
            ..from(0, 4, "hello")
        };
        let line = notice(&server(), &[], &long_name).unwrap();

        assert!(line.starts_with(&"é".repeat(MAX_AUTHOR)));
        assert!(line.ends_with("in #general: hello"));

        let line = notice(&server(), &[], &from(2, 4, &"a".repeat(10_000))).unwrap();

        assert_eq!(
            line.chars().count(),
            "Smiddy in #general: ".len() + MAX_BODY + 1
        );
    }

    #[test]
    fn conversations_are_newest_first_with_ties_by_name_and_no_timestamp_last() {
        let dms = HashMap::from([
            (
                "a".to_string(),
                vec![
                    conversation(1, "Robin", Some(3_000)),
                    conversation(2, "Wren", None),
                    conversation(4, "Ash", Some(2_000)),
                ],
            ),
            ("b".to_string(), vec![conversation(3, "Sam", Some(2_000))]),
        ]);

        let names = |servers: &[crate::model::ServerEntry]| {
            collect_dms(servers, &dms)
                .into_iter()
                .map(|dm| dm.user_name)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            names(&[entry("a", 0), entry("b", 1)]),
            vec!["Robin", "Ash", "Sam", "Wren"]
        );
        assert_eq!(
            names(&[entry("b", 0), entry("a", 1)]),
            names(&[entry("a", 0), entry("b", 1)])
        );
        assert!(collect_dms(&[entry("c", 0)], &dms).is_empty());
    }

    #[test]
    fn store_keys_name_their_entry() {
        assert_eq!(entry_of("abc"), "abc");
        assert_eq!(entry_of(&password_store_key("abc")), "abc");
        assert_eq!(entry_of(&baseline_store_key("abc")), "abc");
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
