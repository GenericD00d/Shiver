//! The core's own connections to every server the webview is not showing (Android has one webview).
//!
//! Each connection keeps that server's unread count (above its floor) and its DM list, and hands
//! what arrives to `notify`. Sessions, passwords and floors live in the encrypted store
//! (`tauri-plugin-shiver-secrets`), written by one background thread in order.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

use shiver_core::{
    jwt::{self, Freshness, Renewals},
    limit::Joins,
    LockExt,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_shiver_secrets::SecretsExt;
use zeroize::Zeroizing;

use crate::{
    sharkord,
    store::{RegistryStore, Store},
    webview,
};

/// Emitted to Shiver's own pages with the per-server unread counts.
pub const INBOX_EVENT: &str = "shiver://inbox";

/// Refusals in a row, each after a fresh sign-in, before Shiver stops trying for a server.
const MAX_REFUSALS: u32 = 3;

/// How long each server left goes unwatched: switching away and back reloads its page, and
/// opening and closing a connection of Shiver's own besides doubled what the server saw.
pub(crate) const WATCH_GRACE: Duration = Duration::from_secs(30);

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
    /// the other person's picture, a signed link on that server's own origin
    pub avatar_url: Option<String>,
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
                    avatar_url: dm.avatar_url.clone(),
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

#[derive(Default)]
struct State {
    /// wiped when replaced or dropped; copies handed out are short-lived
    tokens: HashMap<String, Zeroizing<String>>,
    unread: HashMap<String, u32>,
    running: HashMap<String, tauri::async_runtime::JoinHandle<()>>,
    /// the one pending `sync` for when the next server's `WATCH_GRACE` runs out
    grace_timer: Option<tauri::async_runtime::JoinHandle<()>>,
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
                .insert(entry_id.to_string(), Zeroizing::new(token.to_string()))
                .map_or(true, |previous| previous.as_str() != token)
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

/// A write (`Some`) or removal (`None`) of one store key. Values are sessions and passwords (and
/// floors), so they are wiped once written.
type StoreWrite = (String, Option<Zeroizing<String>>);

/// Queues a write or removal for the one store-writer thread, which keeps writes in order and off
/// the UI thread (each is a blocking call into the JVM).
fn store_off_thread(app: &AppHandle, key: String, value: Option<Zeroizing<String>>) {
    static WRITER: std::sync::OnceLock<std::sync::mpsc::Sender<StoreWrite>> =
        std::sync::OnceLock::new();

    let sender = WRITER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<StoreWrite>();
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

            let Ok(Some(value)) = app
                .shiver_secrets()
                .get(&key)
                .map(|value| value.map(Zeroizing::new))
            else {
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

        // Shiver opens the last server at launch, whose page joins it: the core's own connection
        // waiting out the grace saves a second join, which Sharkord allows only a few of a minute
        let last = app
            .state::<Store>()
            .registry()
            .settings
            .last_server_id
            .clone();

        if let Some(last) = last {
            app.state::<webview::Showing>().hold_back(&last);
        }

        sync(&app);
        sign_in_missing(&app);
    });
}

/// Signs in the servers that have a stored password but no session.
fn sign_in_missing(app: &AppHandle) {
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let missing: Vec<String> = {
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
                .map(|server| server.id.clone())
                .collect()
        };

        let mut signed_in = false;

        for entry_id in missing {
            signed_in |= renew(&handle, &entry_id).await.is_some();
        }

        if signed_in {
            sync(&handle);
        }
    });
}

/// Keeps a session (in memory, and in the store when `persist`); returns whether it was new. A
/// token past `MAX_TOKEN` is not kept.
fn keep_session(app: &AppHandle, entry_id: &str, token: &str, persist: bool) -> bool {
    if token.len() > MAX_TOKEN {
        return false;
    }

    if app
        .state::<Inbox>()
        .with(|state| state.signed_out.remove(entry_id))
    {
        publish(app);
    }

    if !app.state::<Inbox>().remember_token(entry_id, token) {
        return false;
    }

    if persist {
        store_off_thread(
            app,
            entry_id.to_string(),
            Some(Zeroizing::new(token.to_string())),
        );
    }

    true
}

/// Keeps a session and connects with it.
fn record_session(app: &AppHandle, entry_id: &str, token: &str, persist: bool) {
    if keep_session(app, entry_id, token, persist) {
        sync(app);
    }
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
        Some(Zeroizing::new(password.to_string())),
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
    crate::notify::forget(app, entry_id);
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
    app.state::<Joins>().forget(entry_id);
    crate::notify::clear(app, entry_id);

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
        crate::notify::clear(app, entry_id);
    }

    for entry_id in wanted {
        let task = tauri::async_runtime::spawn(watch(app.clone(), entry_id.clone()));

        app.state::<Inbox>()
            .with(|state| state.running.insert(entry_id, task));
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
        let place = self.app.state::<Joins>().take(&self.entry_id);

        tokio::time::sleep(place.wait).await;

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
            sharkord::Event::Posted(message) => {
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

/// The session to open this entry with. One due for renewal is used as it is and renewed in the
/// background; a missing or expired one is renewed first, and an expired one is still used when
/// that fails (the server's refusal then takes its usual course).
pub async fn session_for(app: &AppHandle, entry_id: &str) -> Option<String> {
    let token = app.state::<Inbox>().token(entry_id);

    match token.as_deref().map(jwt::freshness) {
        Some(Freshness::Fresh) => token,
        Some(Freshness::Due) => {
            renew_in_background(app, entry_id);

            token
        }
        Some(Freshness::Expired) | None => renew(app, entry_id).await.or(token),
    }
}

fn renew_in_background(app: &AppHandle, entry_id: &str) {
    if !app.state::<Inbox>().has_password(entry_id) || !app.state::<Renewals>().begin(entry_id) {
        return;
    }

    let (app, entry_id) = (app.clone(), entry_id.to_string());

    tauri::async_runtime::spawn(async move {
        if renew(&app, &entry_id).await.is_some() {
            app.state::<Renewals>().succeeded(&entry_id);
        }
    });
}

/// Signs a server in again from its stored password and keeps the session, without connecting
/// with it (the caller does). A refused password is forgotten.
async fn renew(app: &AppHandle, entry_id: &str) -> Option<String> {
    let (identity, origin) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .and_then(|server| Some((server.identity.clone()?, server.origin.clone())))?;

    if !app.state::<Inbox>().has_password(entry_id) {
        return None;
    }

    let handle = app.clone();
    let key = password_store_key(entry_id);
    let password =
        match tokio::task::spawn_blocking(move || handle.shiver_secrets().get(&key)).await {
            Ok(Ok(Some(password))) => Zeroizing::new(password),
            Ok(Ok(None)) => return None,
            Ok(Err(error)) => {
                eprintln!("[shiver] could not read the stored password for {origin}: {error}");

                return None;
            }
            Err(_) => return None,
        };

    match shiver_core::login::sign_in(&origin, &identity, &password).await {
        Ok(session) => {
            keep_session(app, entry_id, &session, true);

            Some(session)
        }
        Err(error @ shiver_core::Error::Refused(_)) => {
            eprintln!("[shiver] {origin} refused Shiver's stored password: {error}");
            forget_password(app, entry_id);

            None
        }
        Err(error) => {
            eprintln!("[shiver] could not reach {origin} to sign in again: {error}");

            None
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

        // a page seeded before a background renewal still holds the older session
        if handle
            .state::<Inbox>()
            .token(&entry_id)
            .is_some_and(|held| jwt::outlasts(&held, &token))
        {
            return;
        }

        // off this thread: storing calls into the JVM, which deadlocks from an eval callback
        let (handle, id) = (handle.clone(), entry_id.clone());

        std::thread::spawn(move || record_session(&handle, &id, &token, persist));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
            avatar_url: None,
        }
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
