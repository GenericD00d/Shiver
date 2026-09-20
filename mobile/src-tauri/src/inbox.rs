//! The unified inbox on mobile: unread counts from servers that have no page on screen.
//!
//! One webview means one server's client is running at a time, so before this the mobile rail could
//! only ever show a badge for the server you were already looking at — which is no badge at all.
//! The core holds its own connection to every *other* server instead (`sharkord.rs`), counts what
//! arrives, and tells both rails.
//!
//! Three decisions worth knowing:
//!
//! - **The shown server is deliberately not connected.** Its own client is right there doing the
//!   job, and you do not need a badge for the thing you are looking at. So Shiver holds at most
//!   `servers - 1` sockets, and hands the server back to itself the moment you open it.
//! - **Tokens are kept encrypted, not in memory.** `keyring` has no Android backend and a bearer
//!   token in `servers.json` would be a credential written to disk in the clear, so Shiver holds them
//!   in `EncryptedSharedPreferences` through its own small Android plugin — master key in the
//!   Keystore, keys encrypted as well as values. A token still *arrives* by being read out of that
//!   server's own page when the user visits it, but that read is now an opportunity rather than the
//!   only path: once stored, the inbox works from a cold start without visiting anything.
//! - **Muting is applied here.** The server keeps counting a channel this user muted, and is right
//!   to — the mute is Shiver's, on this device. So it is filtered at the last moment.
//! - **One notification per server, replaced as messages arrive.** The same sockets carry the
//!   messages themselves, not only the counts, so Shiver can say who wrote and what they said. It
//!   posts one notification per server and rewrites it — a busy server would otherwise bury the
//!   shade, and what a person needs to know is which server wants them and what the last thing
//!   said. It goes away when they open that server.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shiver_secrets::SecretsExt;

use crate::{
    sharkord,
    store::{RegistryStore, Store},
    webview,
};

/// The event Shiver's own pages listen for. The bridge cannot listen — a page on a server's origin
/// has no IPC — so it is called into instead, see `push_to_page`.
pub const INBOX_EVENT: &str = "shiver://inbox";

/// How long to wait before trying a server that just failed. Long enough not to hammer a server
/// that is down, short enough that a phone coming back onto wifi catches up quickly.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// How much of a name or a message may reach the notification shade, in characters.
///
/// Everything in a notice comes from a server. A five-thousand-character username makes the shade
/// unreadable and pushes the message it was supposed to introduce off the end of it; a very long
/// message body does the same. Android truncates for display, but the string is built, held and
/// handed across the JNI boundary first, so the bound belongs here. Matches the caps desktop's feed
/// uses (`MAX_AUTHOR` / `MAX_BODY` in `feed.rs`), so the two clients say the same thing.
const MAX_AUTHOR: usize = 100;
const MAX_BODY: usize = 500;
const MAX_CHANNEL_NAME: usize = 100;

/// Shortens a string from a server to `limit` characters.
///
/// By characters and not bytes: `String` slicing is by byte offset and panics on a boundary inside
/// a codepoint, so `&text[..limit]` is a crash a server could choose to cause simply by writing in
/// a language that is not ascii. Returns the original untouched when it already fits, which is
/// every real message.
fn clamp(text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }

    // an ellipsis rather than a bare cut, so a truncated line reads as truncated
    text.chars().take(limit).collect::<String>() + "…"
}

/// Where a Sharkord page keeps a token Shiver can use, and — which matters more — which of the two it
/// found.
///
/// `localStorage['sharkord-auto-login-token']` is the persistent one behind Sharkord's own "Login
/// automatically". A user who ticked that asked for their session to outlive the page, so Shiver
/// keeping it in the encrypted store is the same answer to the same question, and it is what makes
/// the inbox work from a cold start.
///
/// `sessionStorage['sharkord-token']` is the live session and nothing more. A user who did *not*
/// tick the box asked for their session to end with the page — so this one is used while Shiver is
/// running and never written to the store. Shiver used to keep both, which was not a hole (same user,
/// same device, encrypted at rest) but did quietly overrule a choice the user had made.
///
/// The answer is prefixed with which storage it came from, so the caller can tell them apart. A
/// bare conditional expression, deliberately: `eval_with_callback` wraps the script to capture its
/// value, and an immediately-invoked function with a `try` in it came back as nothing at all — the
/// same silence as the code never running.
const TOKEN_SCRIPT: &str = "localStorage.getItem('sharkord-auto-login-token') \
     ? 'kept:' + localStorage.getItem('sharkord-auto-login-token') \
     : (sessionStorage.getItem('sharkord-token') \
        ? 'live:' + sessionStorage.getItem('sharkord-token') : null)";

/// One conversation, plus which rail entry it belongs to.
///
/// The shape desktop's inbox uses (`DmEntry` in `desktop/src-tauri/src/feed.rs`), so both clients
/// say the same thing about a conversation: who it is with, which server it is on and which account
/// on that server it belongs to.
///
/// This never crosses into a server's page. It is read by `list_dms`, which only Shiver's own pages
/// can call — they are the one place the user's conversations across every server may be gathered
/// in one list without handing one server the names of the people the user talks to on the others.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmEntry {
    pub entry_id: String,
    pub server_name: String,
    /// which account on that server, shown when one origin was added more than once
    pub account_label: Option<String>,
    pub channel_id: i64,
    pub user_name: String,
    /// when the last message arrived, in milliseconds — the server's own figure, and what this
    /// list is ordered by
    pub last_message_at: Option<u64>,
}

/// Every conversation Shiver knows about, **newest first**.
///
/// Ordered by when the last message arrived and by nothing else. It used to be grouped by server in
/// rail order, which meant the list reshuffled every time the user switched servers — the one they
/// were looking at rose to the top — so the same conversation was never in the same place twice.
/// Which server a conversation is on is written on the row; it is not a reason to sort by it.
///
/// The timestamp is the server's own `max(messages.createdAt)` from `dms.get`, so the order covers
/// each conversation's whole history rather than only the part Shiver was awake for.
///
/// A conversation whose server answered without the field sorts last rather than first: unknown is
/// not the same as recent, and putting it at the top would give the least-known rows the most
/// prominent place. Ties break on the name, so the order is total and cannot wobble between reads.
pub fn collect_dms(
    servers: &[crate::model::ServerEntry],
    dms: &HashMap<String, Vec<sharkord::DirectMessage>>,
) -> Vec<DmEntry> {
    let mut collected: Vec<DmEntry> = servers
        .iter()
        .flat_map(|server| {
            dms.get(&server.id)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
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

/// What one server's notification is currently saying.
#[derive(Debug, Clone, Default)]
struct Announcement {
    /// how many messages it stands for, so a second one can say "and 2 more"
    count: u32,
    /// the most recent line, already written the way it is shown
    latest: String,
}

#[derive(Default)]
pub struct Inbox {
    inner: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// entry id -> session token. Mirrored into the encrypted store, which is the copy that
    /// survives the process; this map is only the one the connections read from.
    tokens: HashMap<String, String>,
    /// entry id -> unread total, already filtered by this user's mutes
    unread: HashMap<String, u32>,
    /// entry id -> the task holding that server's connection
    running: HashMap<String, tauri::async_runtime::JoinHandle<()>>,
    /// entry id -> what Shiver has announced for that server and not yet cleared
    announced: HashMap<String, Announcement>,
    /// entries with a notification already on its way to Android, so a burst is one call
    posting: std::collections::HashSet<String>,
    /// entry id -> that server's direct-message conversations, as of the last connection.
    ///
    /// Kept rather than read on demand, because the server Shiver is *showing* is the one server it
    /// has no connection to — its own client is reporting there instead. Without a cache the list
    /// would be missing whichever server the user happens to be looking at, which is the one they
    /// are most likely to want.
    dms: HashMap<String, Vec<sharkord::DirectMessage>>,
    /// entry id -> what that server was already holding unread when Shiver connected to it.
    ///
    /// The floor the badge is measured from. Sharkord's read state counts every message the user
    /// has never opened the channel to read, which on a public server is its whole history — so
    /// without this the tile says 99+ from the first launch and can never say anything else.
    baselines: HashMap<String, HashMap<i64, u32>>,
    /// entry id -> that server's unread per channel, as of the last thing it said.
    ///
    /// Kept so a badge can be recomputed without the server saying anything new. Muting a channel
    /// changes what the count should be while the connection sits idle, and the read states
    /// otherwise live only inside the task holding that connection.
    read_states: HashMap<String, HashMap<i64, u32>>,
    /// entries whose session was refused and that Shiver cannot sign in again by itself.
    ///
    /// The rail shows these, because until this existed a refused token was *silent*: Shiver dropped
    /// it and said nothing, and the server simply stopped reporting.
    signed_out: std::collections::HashSet<String>,
    /// entries Shiver holds a password for, so it knows without asking the store
    remembered: std::collections::HashSet<String>,
    /// entry id -> what was kept when that server's storage was last wiped.
    ///
    /// Mirrored from the encrypted store for one reason: the page-load hook needs it, and a
    /// blocking call into the JVM from that thread deadlocks the webview — the page never finishes
    /// loading and the previous frame stays on screen, which reads as Shiver hanging on "Connecting".
    carried: HashMap<String, String>,
    /// entry id -> why Shiver cannot watch it, for the servers where that is permanent
    problems: HashMap<String, String>,
    /// entry id -> the companion plugin's version there, or `None` for a server without it.
    ///
    /// **A key is only present once Shiver has connected**, and that distinction is the point: a
    /// missing key means "not asked yet", a `None` means "asked, and it is not installed". The two
    /// must not look the same, or a server Shiver has never reached would be reported as missing
    /// the plugin. It cannot be asked any earlier — see `plugin_version` in `sharkord-client`.
    plugins: HashMap<String, Option<String>>,
}

impl Inbox {
    fn state(&self) -> MutexGuard<'_, State> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records a token read out of a server's own page.
    ///
    /// Returns whether this is new information, so a caller does not restart a healthy connection
    /// every time the page it is watching happens to reload.
    pub fn remember_token(&self, entry_id: &str, token: &str) -> bool {
        let mut state = self.state();

        if state.tokens.get(entry_id).map(String::as_str) == Some(token) {
            return false;
        }

        state.tokens.insert(entry_id.to_string(), token.to_string());

        true
    }

    pub fn forget(&self, entry_id: &str) {
        let mut state = self.state();

        state.tokens.remove(entry_id);
        state.unread.remove(entry_id);
        state.announced.remove(entry_id);
        state.signed_out.remove(entry_id);
        state.remembered.remove(entry_id);
        state.dms.remove(entry_id);
        state.read_states.remove(entry_id);
        state.baselines.remove(entry_id);

        if let Some(task) = state.running.remove(entry_id) {
            task.abort();
        }
    }

    /// What was kept from this entry's last visit, read from memory so the page-load hook never
    /// has to wait on the plugin.
    /// Records a problem. Answers whether this is the first time it has been seen this run,
    /// which is what stops the thirty-second retry turning into a notification every thirty seconds.
    pub fn remember_problem(&self, entry_id: &str, reason: &str) -> bool {
        let mut state = self.state();
        let first = !state.problems.contains_key(entry_id);

        state
            .problems
            .insert(entry_id.to_string(), reason.to_string());

        first
    }

    /// Forgets a problem, because the connection this call follows is proof it is over.
    pub fn clear_problem(&self, entry_id: &str) {
        self.state().problems.remove(entry_id);
    }

    /// Records what the join payload said about the companion plugin.
    fn remember_plugin(&self, entry_id: &str, version: Option<String>) {
        self.state().plugins.insert(entry_id.to_string(), version);
    }

    /// Every server Shiver has connected to, and the plugin version it found there.
    pub fn plugins(&self) -> Vec<(String, Option<String>)> {
        self.state()
            .plugins
            .iter()
            .map(|(entry_id, version)| (entry_id.clone(), version.clone()))
            .collect()
    }

    pub fn problems(&self) -> Vec<(String, String)> {
        self.state()
            .problems
            .iter()
            .map(|(id, reason)| (id.clone(), reason.clone()))
            .collect()
    }

    pub fn carried(&self, entry_id: &str) -> Option<String> {
        self.state().carried.get(entry_id).cloned()
    }

    pub fn remember_carried(&self, entry_id: &str, carried: &str) {
        self.state()
            .carried
            .insert(entry_id.to_string(), carried.to_string());
    }

    /// One server's session, for handing to that server's own page and nowhere else.
    pub fn token(&self, entry_id: &str) -> Option<String> {
        self.state().tokens.get(entry_id).cloned()
    }

    /// Every server's conversations, newest connection wins.
    pub fn dms(&self) -> HashMap<String, Vec<sharkord::DirectMessage>> {
        self.state().dms.clone()
    }

    pub fn remember_dms(&self, entry_id: &str, dms: Vec<sharkord::DirectMessage>) {
        let mut state = self.state();

        if dms.is_empty() {
            // a server with nothing to say about conversations has not necessarily lost the ones it
            // reported last time; an empty answer replaces nothing
            return;
        }

        state.dms.insert(entry_id.to_string(), dms);
    }

    pub fn unread(&self) -> HashMap<String, u32> {
        self.state().unread.clone()
    }

    /// Servers waiting for the user to sign in, which is the one thing Shiver cannot do for them.
    pub fn signed_out(&self) -> Vec<String> {
        self.state().signed_out.iter().cloned().collect()
    }

    fn mark_signed_out(&self, entry_id: &str) -> bool {
        self.state().signed_out.insert(entry_id.to_string())
    }

    /// Drops the session and what was counted with it, keeping everything that outlives one.
    fn forget_token(&self, entry_id: &str) {
        let mut state = self.state();

        state.tokens.remove(entry_id);
        state.unread.remove(entry_id);
        state.announced.remove(entry_id);
        state.read_states.remove(entry_id);

        // **The baseline stays.** It used to go with the session, which meant a token expiring
        // marked that server read: the next connection found no floor, took a fresh one against
        // everything the server was holding, and the badge came back at zero. A session ending is
        // not the user reading anything — only opening the server is, and `sync` is where that is
        // handled. Same mistake as re-taking the baseline on every reconnect.

        if let Some(task) = state.running.remove(entry_id) {
            task.abort();
        }
    }

    /// Notes that this entry can be signed in again without asking anybody.
    pub fn remember_signed_in(&self, entry_id: &str) {
        let mut state = self.state();

        state.remembered.insert(entry_id.to_string());
        state.signed_out.remove(entry_id);
    }

    pub fn forget_remembered(&self, entry_id: &str) {
        self.state().remembered.remove(entry_id);
    }

    /// This server's unread floor — what was already sitting unread when Shiver first saw it.
    ///
    /// Handed to a page so it can put it where this user's other devices will find it. **The
    /// floor, not the current counts**: publishing the counts would move the floor up to meet them
    /// every time the server was opened, which is the same as marking everything read on arrival.
    /// Writing the same floor repeatedly is harmless, and it is how a server that had the plugin
    /// installed later catches up without anything having to notice.
    pub fn baseline(&self, entry_id: &str) -> Option<HashMap<i64, u32>> {
        self.state().baselines.get(entry_id).cloned()
    }

    pub fn has_password(&self, entry_id: &str) -> bool {
        self.state().remembered.contains(entry_id)
    }

    /// Clears the "sign in" flag, which is what a fresh session means.
    pub fn clear_signed_out(&self, entry_id: &str) -> bool {
        self.state().signed_out.remove(entry_id)
    }

    /// Folds one more message into what this server's notification says.
    fn announce(&self, entry_id: &str, line: String) -> Announcement {
        let mut state = self.state();
        let announcement = state.announced.entry(entry_id.to_string()).or_default();

        announcement.count += 1;
        announcement.latest = line;

        announcement.clone()
    }

    /// Forgets what was announced for a server, so the next message starts a fresh count.
    fn silence(&self, entry_id: &str) -> bool {
        self.state().announced.remove(entry_id).is_some()
    }

    /// Claims the right to post this server's notification, if nobody else already holds it.
    fn claim_posting(&self, entry_id: &str) -> bool {
        self.state().posting.insert(entry_id.to_string())
    }

    /// Gives it up, and hands back whatever the notification should now say.
    fn release_posting(&self, entry_id: &str) -> Option<Announcement> {
        let mut state = self.state();

        state.posting.remove(entry_id);

        state.announced.get(entry_id).cloned()
    }

    fn set_unread(&self, entry_id: &str, count: u32) -> bool {
        let mut state = self.state();

        if state.unread.get(entry_id) == Some(&count) {
            return false;
        }

        state.unread.insert(entry_id.to_string(), count);

        true
    }
}

/// Loads the sessions kept from previous runs and starts watching with them.
///
/// This is what makes the inbox work on a cold start: before it existed, Shiver only had a token for
/// a server the user had already opened in this run, so a freshly launched client knew nothing about
/// any server until it was visited. Keys are entry ids; a key with no entry behind it belonged to a
/// server that has since been removed, and is deleted rather than left lying around encrypted.
pub fn restore(app: &AppHandle) {
    let handle = app.clone();

    // off the main thread for the reason on the harvest above: reading the store is a blocking
    // call into the JVM, and `setup` runs on the thread the JVM would have to answer on.
    std::thread::spawn(move || restore_now(&handle));
}

fn restore_now(app: &AppHandle) {
    let keys = match app.shiver_secrets().keys() {
        Ok(keys) => keys,
        Err(error) => {
            eprintln!("[shiver] could not read the session store: {error}");

            return;
        }
    };

    let known: Vec<String> = {
        let store = app.state::<Store>();
        let registry = store.registry();

        registry
            .servers
            .iter()
            .map(|server| server.id.clone())
            .collect()
    };

    let mut restored = 0;

    for key in keys {
        // the store holds two kinds of key and both are named after an entry: the session itself,
        // and what was carried across that entry's last storage wipe. Reading the entry id out of
        // either is what tells a key belonging to a removed server from one still in use — without
        // it every carried value looks orphaned and gets deleted on the next launch.
        let entry_id = entry_of(&key);

        if !known.iter().any(|known| known == entry_id) {
            let _ = app.shiver_secrets().remove(&key);

            continue;
        }

        let Ok(Some(value)) = app.shiver_secrets().get(&key) else {
            continue;
        };

        if key.starts_with(BASELINE_PREFIX) {
            // A shape Shiver wrote itself, so a value it cannot read is a value from an older
            // build or a corrupted store — dropped rather than guessed at, which costs one
            // launch's badge and not the connection.
            match serde_json::from_str::<HashMap<i64, u32>>(&value) {
                Ok(baseline) => {
                    app.state::<Inbox>()
                        .state()
                        .baselines
                        .insert(entry_id.to_string(), baseline);
                }
                Err(error) => {
                    eprintln!(
                        "[shiver] could not read the stored baseline for {entry_id}: {error}"
                    );

                    let _ = app.shiver_secrets().remove(&key);
                }
            }
        } else if key.starts_with(CARRIED_PREFIX) {
            app.state::<Inbox>().remember_carried(entry_id, &value);
        } else if key.starts_with(PASSWORD_PREFIX) {
            // Left where it is. A password is read at the moment it is needed and never held in
            // memory: it is wanted about once a week, and the cost of fetching it then is one call
            // into the store against a secret that would otherwise sit in the process all day.
            app.state::<Inbox>().remember_signed_in(entry_id);
        } else {
            app.state::<Inbox>().remember_token(entry_id, &value);
            restored += 1;
        }
    }

    if restored > 0 {
        eprintln!("[shiver] restored {restored} session(s) from the encrypted store");

        sync(app);
    }

    // and then the ones no session came back for, which is the whole point of holding a password
    sign_in_missing(app);
}

/// Signs in, at launch, every server that has a password but no session.
///
/// `restore_now` above brings back the sessions that were stored, and `sync` watches those. A server
/// whose session expired since the last launch — Sharkord's last a week and cannot be refreshed —
/// has none, so it is not in `tokens`, so `sync` never considers it, so no watch is started for it.
/// The watch loop is the one thing that signs a server back in, which left the server most in need
/// of a sign-in as the only one that could not get one: it stayed dark, with no badge and no
/// notifications, until the user happened to open it and the page handed a fresh token over.
///
/// That is the whole gap. Everything else about a server is watched from launch already.
///
/// One at a time rather than a task each: a launch is already contending for the radio with the
/// client the user is waiting to see, and these are a handful of small POSTs whose answers are not
/// needed in any particular order. `sign_in_again` stores the session and calls `sync` itself, so a
/// server starts being watched as its own sign-in lands rather than after the slowest one.
fn sign_in_missing(app: &AppHandle) {
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let missing: Vec<(String, String)> = {
            let inbox = handle.state::<Inbox>();
            let state = inbox.state();
            let store = handle.state::<Store>();
            let registry = store.registry();

            registry
                .servers
                .iter()
                .filter(|server| {
                    !state.tokens.contains_key(&server.id)
                        && state.remembered.contains(&server.id)
                        && server.identity.is_some()
                })
                .map(|server| (server.id.clone(), server.origin.clone()))
                .collect()
        };

        if missing.is_empty() {
            return;
        }

        eprintln!(
            "[shiver] signing in to {} server(s) with no session",
            missing.len()
        );

        for (entry_id, origin) in missing {
            sign_in_again(&handle, &entry_id, &origin).await;
        }
    });
}

/// Records a session and starts watching with it, storing it where the next launch will find it.
///
/// The one way a token enters Shiver, whether it came from signing in at the add screen or from being
/// read out of a page later. Both need the same three things to happen, and only one of them should
/// know the order.
pub fn remember_session(app: &AppHandle, entry_id: &str, token: &str) {
    record_session(app, entry_id, token, true);
}

/// The same, for a session that is the user's for this run only.
///
/// A token from `sessionStorage` belongs to a user who did not ask Sharkord to keep them signed in.
/// Shiver uses it — it is what the inbox watches that server with while the app is up — and does not
/// write it to the encrypted store, so the session ends when they meant it to.
///
/// It leaves whatever is already stored alone rather than clearing it. Anything in there is a token
/// the user *did* opt into keeping, on a visit where the box was ticked; it is not this function's
/// to throw away, and it expires by itself within the week.
pub fn remember_session_for_this_run(app: &AppHandle, entry_id: &str, token: &str) {
    record_session(app, entry_id, token, false);
}

fn record_session(app: &AppHandle, entry_id: &str, token: &str, persist: bool) {
    // A session is the answer to "sign in", however it arrived: from the add screen, from this
    // server's own page, or from Shiver signing in again by itself. So the flag goes first, and
    // before the early return below — a token Shiver already had still settles the question.
    if app.state::<Inbox>().clear_signed_out(entry_id) {
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

/// Writes a session to the encrypted store from a thread of Shiver's own.
///
/// Every caller goes through here, and not for tidiness. `run_mobile_plugin` blocks the calling
/// thread until the JVM answers, and the JVM answers on the Android main thread — so any plugin call
/// made *from* the main thread waits on a reply only it could deliver. That is a hang, not a slow
/// path, and it has arrived three separate ways now: from an eval callback, from the page-load hook,
/// and from a synchronous command handler. Nothing that reaches the store is urgent enough to be
/// worth being on the caller's thread, so none of it is.
fn store_off_thread(app: &AppHandle, key: String, value: Option<String>) {
    let handle = app.clone();

    std::thread::spawn(move || {
        let result = match &value {
            Some(value) => handle.shiver_secrets().set(&key, value),
            None => handle.shiver_secrets().remove(&key),
        };

        if let Err(error) = result {
            eprintln!("[shiver] could not update the stored session for {key}: {error}");
        }
    });
}

/// Where a server's carried page state lives in the encrypted store.
///
/// Kept here beside the session keys so one module owns the shape of that store.
pub fn carried_store_key(entry_id: &str) -> String {
    format!("{CARRIED_PREFIX}{entry_id}")
}

/// What marks a store key as carried page state rather than a session.
const CARRIED_PREFIX: &str = "carried:";

/// Where a server's unread baseline lives, and what marks the key as one.
///
/// **Persisted, and that is the whole point.** The baseline is the floor the badge is measured
/// from, and while it lived only in memory every restart took a fresh one against whatever the
/// server currently said — so closing Shiver silently marked everything read, and the badge came
/// back at zero however much had arrived. Kept across launches, the same floor still applies, so
/// the count is "since you last opened this server" rather than "since this process started",
/// including everything that arrived while Shiver was not running.
///
/// It goes in the encrypted store because that is the store mobile has, not because a count of
/// unread messages is a secret.
pub fn baseline_store_key(entry_id: &str) -> String {
    format!("{BASELINE_PREFIX}{entry_id}")
}

const BASELINE_PREFIX: &str = "baseline:";

/// What marks a store key as a password the user asked Shiver to keep.
///
/// Kept only where the user has said so, per server. A session token is worth seven days —
/// Sharkord signs one `expiresIn: '604800s'` and offers no way to refresh it — so without a
/// password Shiver's watch of a server dies weekly and can only be revived by opening that server.
/// With one, Shiver signs in again by itself and the server never goes quiet.
const PASSWORD_PREFIX: &str = "password:";

/// Where a server's password lives, for the servers that have one.
pub fn password_store_key(entry_id: &str) -> String {
    format!("{PASSWORD_PREFIX}{entry_id}")
}

/// The entry a store key belongs to, whichever kind of key it is.
fn entry_of(key: &str) -> &str {
    key.strip_prefix(CARRIED_PREFIX)
        .or_else(|| key.strip_prefix(PASSWORD_PREFIX))
        .or_else(|| key.strip_prefix(BASELINE_PREFIX))
        .unwrap_or(key)
}

/// Forgets a server's session everywhere: the live map, the connection, and the encrypted store.
///
/// Separate from `Inbox::forget`, which only knows about memory. Anything that ends a session —
/// removing the server, signing out, a token the server refuses — has to reach the store too, or
/// the next launch would restore the session that was just discarded.
pub fn forget_everywhere(app: &AppHandle, entry_id: &str) {
    // memory first and inline: the connection has to stop before anything can reconnect on the
    // session being discarded, and that decision is made from the map this clears
    app.state::<Inbox>().forget(entry_id);

    store_off_thread(app, entry_id.to_string(), None);
    store_off_thread(app, carried_store_key(entry_id), None);
    store_off_thread(app, password_store_key(entry_id), None);
    store_off_thread(app, baseline_store_key(entry_id), None);
}

/// Brings the set of live connections in line with what Shiver should be watching.
///
/// Called whenever anything that decides that changes: a token arrives, a server is opened or
/// closed, a server is removed. Idempotent, so callers never have to work out what changed.
pub fn sync(app: &AppHandle) {
    let (wanted, unwanted) = {
        let inbox = app.state::<Inbox>();
        let state = inbox.state();
        let store = app.state::<Store>();
        let registry = store.registry();

        let mut wanted = Vec::new();
        let mut unwanted = Vec::new();

        for entry_id in state.tokens.keys() {
            let known = registry.servers.iter().any(|server| server.id == *entry_id);

            // **The server on screen keeps its socket too**, which it did not used to.
            //
            // It stood down on the reasoning that its own client is right there reporting, and you
            // do not need a badge for the thing you are looking at. The first half turned out to be
            // false in the way that matters: reading a channel is published by the server to the
            // user's *other sessions*, and this socket is one of them. Hanging it up meant hanging
            // up on the one server whose reads Shiver most needed to hear, so a badge cleared by
            // reading only came right on the way out.
            //
            // What the socket must not do for that server is *announce* — see `announce`, which
            // stays quiet for it. A notification about a message already on screen is noise.
            if !known {
                if state.running.contains_key(entry_id) {
                    unwanted.push(entry_id.clone());
                }

                continue;
            }

            if !state.running.contains_key(entry_id) {
                if let Some(server) = registry.servers.iter().find(|s| s.id == *entry_id) {
                    wanted.push((entry_id.clone(), server.origin.clone()));
                }
            }
        }

        // **The floor is not touched here, and that is deliberate.** Opening a server used to drop
        // it, so the next connection took a fresh one and the badge came back at zero — walk into a
        // server with five unread and they were gone whether or not anything had been read.
        //
        // The floor answers one question only: what was already sitting unread the first time
        // Shiver ever saw this server. On a public server that is its whole history, and without a
        // floor the tile would say 99+ forever. It is not a record of the user glancing at
        // anything. What the badge counts down is Sharkord's own read state, which falls as
        // channels are actually read — so five unread becomes four when one of them is read, and
        // stays at five until then.

        (wanted, unwanted)
    };

    for entry_id in unwanted {
        let inbox = app.state::<Inbox>();
        let mut state = inbox.state();

        if let Some(task) = state.running.remove(&entry_id) {
            task.abort();
        }

        // the badge goes with the connection: a count Shiver is no longer being told about is a
        // count that will be wrong within a minute
        state.unread.remove(&entry_id);

        drop(state);

        clear_notification(app, &entry_id);
    }

    // **Opening a server takes its notification out of the shade**, and this is now the only thing
    // that does it. It used to fall out of the loop above: the shown server's socket stood down, so
    // it was "unwanted", so its notification was cleared on the way past. The socket stays up now,
    // so that no longer happens and the shade would keep a notice about the server on screen.
    if let Some(entry_id) = app.state::<webview::Showing>().server() {
        app.state::<Inbox>().silence(&entry_id);
        clear_notification(app, &entry_id);
    }

    for (entry_id, origin) in wanted {
        let handle = app.clone();
        let id = entry_id.clone();

        let task = tauri::async_runtime::spawn(async move {
            watch(handle, id, origin).await;
        });

        app.state::<Inbox>().state().running.insert(entry_id, task);
    }

    publish(app);
}

/// Holds one server's connection open, reconnecting for as long as Shiver still wants it.
///
/// Before debugging anything in this module: **on Android only these async-runtime tasks can be
/// heard.** Neither the page-load hook, nor the `eval_with_callback` closures, nor even command
/// handlers have their stderr on the pipe Tauri redirects into logcat, so a print in any of them
/// looks exactly like code that never ran. Two evenings went into "this never executes" that was
/// really "this cannot speak". Judge those paths by what they cause — whether a connection starts —
/// and put any new telemetry here.
async fn watch(app: AppHandle, entry_id: String, origin: String) {
    loop {
        let Some(token) = app.state::<Inbox>().state().tokens.get(&entry_id).cloned() else {
            // the token was forgotten while this task was sleeping; nothing more to do
            return;
        };

        // read each time round rather than captured: turning the limit off is meant to take
        // effect on the next attempt, not on the next launch
        let accept_any_size = {
            let store = app.state::<Store>();
            let registry = store.registry();

            registry
                .servers
                .iter()
                .find(|server| server.id == entry_id)
                .is_some_and(|server| server.accept_any_size)
        };

        match sharkord::open(&origin, &token, accept_any_size).await {
            Ok(mut session) => {
                let mut read_states = session.joined.read_states.clone();

                // What the server was already holding when Shiver first arrived. Everything the
                // badge reports is measured against this, so a backlog nobody has read is not news.
                //
                // **Only if there is not one already.** A watch starts every time its server stops
                // being the one on screen, so re-taking this on every connection meant switching to
                // another server and back silently folded the unread into the baseline and put the
                // badge to zero — which is exactly what it looked like from the outside: badges
                // worked, then stopped after a switch, and never came back until new messages
                // arrived. Cleared deliberately in `sync` when the user opens the server, which is
                // the one moment "you have seen this" is actually true.
                // The floor the companion plugin holds for this user on this server, where there
                // is one. It wins over whatever this device stored: it is the same user's floor,
                // kept against their account, so a server read on the desktop is read here too.
                // That is the whole reason it lives on the server rather than on each device.
                let shared = session.joined.shared_floor.clone();

                let took_baseline = {
                    let inbox = app.state::<Inbox>();
                    let mut state = inbox.state();

                    match shared {
                        Some(shared) => {
                            let changed = state.baselines.get(&entry_id) != Some(&shared);

                            state.baselines.insert(entry_id.clone(), shared);

                            // written back to this device's store so a launch with no network, or
                            // with the plugin since removed, still measures from the right place
                            changed
                        }
                        None => {
                            let before = state.baselines.len();

                            state
                                .baselines
                                .entry(entry_id.clone())
                                .or_insert_with(|| read_states.clone());

                            state.baselines.len() != before
                        }
                    }
                };

                // Written out only when this connection is the one that established it. Re-writing
                // an existing baseline on every reconnect would be pointless traffic to the store,
                // and `or_insert_with` above is what decides whether there was one.
                if took_baseline {
                    let floor = app
                        .state::<Inbox>()
                        .state()
                        .baselines
                        .get(&entry_id)
                        .cloned()
                        .unwrap_or_else(|| read_states.clone());

                    persist_baseline(&app, &entry_id, &floor);
                }

                // whatever was wrong before, this connection is proof it is not wrong now
                app.state::<Inbox>().clear_problem(&entry_id);

                // said out loud, like the browser hand-off: a server quietly missing from the inbox
                // is the hardest kind of bug to see, because nothing appears rather than something
                // appearing wrong
                eprintln!(
                    "[shiver] watching {origin} from the core, {} channels",
                    read_states.len()
                );

                // Kept for Shiver's own direct-message screen, which asks for it through `list_dms`.
                // It is deliberately not pushed anywhere: a server's page is never handed the
                // conversations the user has on any other server.
                app.state::<Inbox>()
                    .remember_dms(&entry_id, session.joined.dms.clone());

                // whether this server has Shiver's companion plugin, which arrives with the join
                // and is not askable any other way as an ordinary member
                app.state::<Inbox>()
                    .remember_plugin(&entry_id, session.joined.plugin_version.clone());

                recount(&app, &entry_id, &read_states);

                while let Some(event) = session.next_event().await {
                    match event {
                        sharkord::Event::Unread { channel_id, delta } => {
                            sharkord::apply_delta(&mut read_states, channel_id, delta);
                            recount(&app, &entry_id, &read_states);
                        }
                        // The user read that channel — here, or on the desktop, or in a browser.
                        // It arrives on `channels.onReadStateUpdate`, a different subscription from
                        // the one that carries arrivals, and not subscribing to it was why a badge
                        // could not be cleared by reading the thing that caused it.
                        sharkord::Event::UnreadSet { channel_id, count } => {
                            sharkord::set_unread(&mut read_states, channel_id, count);
                            recount(&app, &entry_id, &read_states);
                        }
                        sharkord::Event::Posted(message) => {
                            announce(&app, &entry_id, &session.joined, &message);
                        }
                    }
                }
            }
            // A token the server rejects will be rejected again in thirty seconds and every
            // thirty seconds after that, so it is not kept. What happens next depends on whether
            // the user asked Shiver to remember their password for this server.
            Err(sharkord::Error::Refused(reason)) => {
                eprintln!("[shiver] {origin} refused Shiver's stored session ({reason})");

                if sign_in_again(&app, &entry_id, &origin).await {
                    // straight back round with the new session rather than waiting out the retry
                    continue;
                }

                // Only the session goes. The password, if the user asked Shiver to keep one, has not
                // been refused — and what the page carried across its last wipe is not a session's
                // business either.
                forget_session(&app, &entry_id);

                return;
            }
            // The one failure that will not come right by itself. Retrying is still correct —
            // a server that trims what it sends, or a Shiver with a bigger limit, fixes it without
            // anyone restarting anything — but it is reported rather than only logged, because
            // otherwise that server simply stops existing as far as notifications are concerned.
            Err(sharkord::Error::TooLarge { size, max }) => {
                eprintln!(
                    "[shiver] {origin} sent {size} bytes in one message and Shiver accepts {max}, so it is not being watched"
                );

                report_watch_problem(&app, &entry_id, size);
            }
            Err(error) => {
                // said out loud: a server silently missing from the inbox is the hardest kind of
                // bug to notice, because nothing appears rather than something appearing wrong
                eprintln!("[shiver] could not watch {origin}: {error}");
            }
        }

        tokio::time::sleep(RETRY_AFTER).await;
    }
}

/// A byte count as a person would say it.
fn megabytes(bytes: usize) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);

    if mb < 1.0 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{mb:.1} MB")
    }
}

/// Records that Shiver cannot watch a server, and says so once.
///
/// Once per server per run: the watch loop comes back every thirty seconds and a notification on
/// every attempt would be its own fault. The record outlives the notification, so the settings
/// screen can still show it after the shade is cleared.
fn report_watch_problem(app: &AppHandle, entry_id: &str, size: usize) {
    let name = {
        let store = app.state::<Store>();
        let registry = store.registry();

        registry
            .servers
            .iter()
            .find(|server| server.id == entry_id)
            .map(|server| server.name.clone())
    };

    let Some(name) = name else {
        return;
    };

    // The row this appears on is already labelled with the server's name, and the person
    // reading it wants to know what is wrong rather than how many bytes it was.
    let first = app.state::<Inbox>().remember_problem(
        entry_id,
        &format!(
            "Sends {} in one message, which is more than Shiver accepts — so nothing from this server reaches this phone.",
            megabytes(size)
        ),
    );

    // the name is the notification's, and the notification is Android's
    #[cfg(not(target_os = "android"))]
    let _ = &name;

    // The record above is what the settings screen reads, on either platform. The shade is
    // Android's alone — the desktop build of this crate exists to run the tests — and only the
    // first sighting says anything, because the watch loop comes back every thirty seconds.
    #[cfg(not(target_os = "android"))]
    let _ = first;

    #[cfg(target_os = "android")]
    if first {
        use tauri_plugin_notification::NotificationExt;

        if let Err(error) = app
            .notification()
            .builder()
            .id(notification_id(&format!("problem:{entry_id}")))
            .title(format!("Shiver cannot watch {name}"))
            .body("This server sends more in one message than Shiver accepts, so its messages will not reach you here. Settings has the details.")
            .show()
        {
            eprintln!("[shiver] could not show the watch problem: {error}");
        }
    }
}

/// Puts a message on the phone's shade, or decides it is not worth saying.
///
/// The rules are Shiver's own, because there is no page here to make them: Sharkord's client decides
/// what deserves a notification from settings that live in that page's storage, and Shiver has no
/// page for a server it is only watching. So this keeps to what Shiver can defend — nothing the user
/// wrote themselves, nothing in a channel they muted, and one notification per server.
fn announce(
    app: &AppHandle,
    entry_id: &str,
    joined: &sharkord::Joined,
    message: &sharkord::NewMessage,
) {
    // The server on screen keeps a socket now, for the reads it is the only thing that hears — but
    // it must not also put messages in the shade. The user is looking at this server; its own
    // client is showing them the message this would be about.
    if app.state::<webview::Showing>().server().as_deref() == Some(entry_id) {
        return;
    }

    let (muted, server) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let muted: Vec<i64> = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == entry_id)
            .map(|muted| muted.channel_id)
            .collect();

        let server = registry
            .servers
            .iter()
            .find(|server| server.id == entry_id)
            .map(|server| server.name.clone());

        (muted, server)
    };

    // a server Shiver no longer has: removed while this connection was still open
    let Some(server) = server else {
        return;
    };

    let Some(line) = notice(joined, &muted, message) else {
        return;
    };

    app.state::<Inbox>().announce(entry_id, line);

    show(app, entry_id, &server);
}

/// The line a message deserves on the shade, or `None` when it deserves nothing.
///
/// Separated from the posting so the rules can be read and tested on their own — they are the part
/// with judgement in them, and the part a person will want to argue with.
fn notice(
    joined: &sharkord::Joined,
    muted: &[i64],
    message: &sharkord::NewMessage,
) -> Option<String> {
    // a person's own message, arriving back down their own socket
    if message.user_id.is_some() && message.user_id == joined.own_user_id {
        return None;
    }

    if muted.contains(&message.channel_id) {
        return None;
    }

    let author = clamp(
        message
            .plugin_id
            .clone()
            .or_else(|| {
                message
                    .user_id
                    .and_then(|id| joined.user_names.get(&id).cloned())
            })
            // a name Shiver has never seen: someone who joined after this connection did
            .unwrap_or_else(|| "Someone".into()),
        MAX_AUTHOR,
    );

    // an image or a file on its own, which is still worth being told about
    let text = if message.text.is_empty() {
        "Sent an attachment".to_string()
    } else {
        clamp(message.text.clone(), MAX_BODY)
    };

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

/// How long a server's notification waits before it is posted, so a burst of messages is one call.
const NOTIFY_AFTER: Duration = Duration::from_millis(900);

/// The notification itself.
///
/// Off this thread, and at most one in flight per server. Showing one is a call into the JVM, which
/// answers on **Android's main thread** and blocks the caller until it does — so a server where ten
/// messages land at once would put ten of those in front of the thread that also has to draw the
/// app and handle every touch. It waits instead, then posts what the notification should say by
/// then, which is the same line the tenth message would have written anyway.
fn show(app: &AppHandle, entry_id: &str, server: &str) {
    if !app.state::<Inbox>().claim_posting(entry_id) {
        // one is already on its way, and it will pick up what has arrived since
        return;
    }

    let app = app.clone();
    let entry_id = entry_id.to_string();
    let title = server.to_string();

    std::thread::spawn(move || {
        std::thread::sleep(NOTIFY_AFTER);

        let Some(announcement) = app.state::<Inbox>().release_posting(&entry_id) else {
            // the user opened the server while this was waiting, so there is nothing left to say
            return;
        };

        post(&app, notification_id(&entry_id), title, &announcement);
    });
}

/// The call into Android, once the waiting is done.
fn post(app: &AppHandle, id: i32, title: String, announcement: &Announcement) {
    use tauri_plugin_notification::NotificationExt;

    let body = if announcement.count > 1 {
        format!(
            "{}\n… and {} more",
            announcement.latest,
            announcement.count - 1
        )
    } else {
        announcement.latest.clone()
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

/// Takes a server's notification off the shade, which is what opening it means.
///
/// Taking one back is Android's alone — the desktop build of this crate exists to run the tests, and
/// there is nothing to take back there, so the call is compiled only where it exists.
fn clear_notification(app: &AppHandle, entry_id: &str) {
    // nothing was announced for this server, so there is nothing on the shade to take back
    let announced = app.state::<Inbox>().silence(entry_id);

    // and a notification still waiting to be posted must not appear after the user went to look
    app.state::<Inbox>().state().posting.remove(entry_id);

    #[cfg(mobile)]
    if announced {
        use tauri_plugin_notification::NotificationExt;

        let app = app.clone();
        let id = notification_id(entry_id);

        std::thread::spawn(move || {
            let _ = app.notification().remove_active(vec![id]);
        });
    }

    #[cfg(not(mobile))]
    let _ = announced;
}

/// A stable notification id for one server, so a second message replaces the first rather than
/// stacking on it. Android wants an `i32`; entry ids are uuids, so this is their hash.
fn notification_id(entry_id: &str) -> i32 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    entry_id.hash(&mut hasher);

    // the sign is meaningless to Android, but a positive id is easier to read in a log
    (hasher.finish() as u32 & 0x7fff_ffff) as i32
}

/// Signs a server in again, where the user has asked Shiver to be able to.
///
/// Sharkord's tokens last a week and cannot be refreshed, so this is what stops a watched server
/// going quiet: Shiver holds the password for the servers the user ticked, reads it at the moment it
/// is needed, and takes a fresh session.
///
/// Off the async runtime to read the secret, because that read is a blocking call into the JVM and
/// this is running on a runtime thread that other servers' connections share.
async fn sign_in_again(app: &AppHandle, entry_id: &str, origin: &str) -> bool {
    let Some(identity) = ({
        let store = app.state::<Store>();
        let registry = store.registry();

        registry
            .servers
            .iter()
            .find(|server| server.id == entry_id)
            .and_then(|server| server.identity.clone())
    }) else {
        return false;
    };

    let password = {
        let handle = app.clone();
        let key = password_store_key(entry_id);

        match tokio::task::spawn_blocking(move || handle.shiver_secrets().get(&key)).await {
            Ok(Ok(Some(password))) => password,
            // no password kept for this server, which is the default and not an error
            Ok(Ok(None)) => return false,
            Ok(Err(error)) => {
                eprintln!("[shiver] could not read the stored password for {origin}: {error}");

                return false;
            }
            Err(error) => {
                eprintln!("[shiver] the password read for {origin} did not finish: {error}");

                return false;
            }
        }
    };

    match crate::login::sign_in(origin, &identity, &password).await {
        Ok(session) => {
            eprintln!("[shiver] signed {origin} back in from the stored password");

            remember_session(app, entry_id, &session);

            true
        }
        // A password the *server* refuses will be refused again, so Shiver stops using it and asks
        // instead — the one thing it cannot do for itself.
        Err(error @ crate::error::Error::Refused(_)) => {
            eprintln!("[shiver] {origin} refused Shiver's stored password: {error}");

            forget_password(app, entry_id);

            false
        }
        // A server that cannot be reached has said nothing about the password, and throwing it away
        // over a lost connection would turn a bad minute into a sign-in the user has to redo.
        Err(error) => {
            eprintln!("[shiver] could not reach {origin} to sign in again: {error}");

            false
        }
    }
}

/// Drops the session Shiver was refused, and says so where the user will see it.
///
/// Narrower than `forget_everywhere` on purpose: this is a session that expired, not a server going
/// away, so the password and the carried page state both stay.
pub fn forget_session(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>().forget_token(entry_id);

    store_off_thread(app, entry_id.to_string(), None);

    if app.state::<Inbox>().mark_signed_out(entry_id) {
        publish(app);
    }
}

/// Keeps a password for a server, which is what lets Shiver sign it in again unattended.
pub fn remember_password(app: &AppHandle, entry_id: &str, password: &str) {
    app.state::<Inbox>().remember_signed_in(entry_id);

    store_off_thread(
        app,
        password_store_key(entry_id),
        Some(password.to_string()),
    );
}

/// Writes a server's unread floor to the store, so the next launch measures from the same place.
fn persist_baseline(app: &AppHandle, entry_id: &str, baseline: &HashMap<i64, u32>) {
    match serde_json::to_string(baseline) {
        Ok(encoded) => store_off_thread(app, baseline_store_key(entry_id), Some(encoded)),
        // costs this server a persisted floor and nothing else; the in-memory one still works
        Err(error) => eprintln!("[shiver] could not encode the baseline for {entry_id}: {error}"),
    }
}

/// Forgets one, either because the user asked or because the server stopped accepting it.
pub fn forget_password(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>().forget_remembered(entry_id);

    store_off_thread(app, password_store_key(entry_id), None);
}

/// Recomputes one server's badge from its read states, the baseline it joined on, and the mutes.
fn recount(app: &AppHandle, entry_id: &str, read_states: &HashMap<i64, u32>) {
    app.state::<Inbox>()
        .state()
        .read_states
        .insert(entry_id.to_string(), read_states.clone());

    let muted = {
        let store = app.state::<Store>();
        let registry = store.registry();

        registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == entry_id)
            .map(|muted| muted.channel_id)
            // a set rather than a list: `unread_total` looks each channel up against this once per
            // channel, and a slice made that a scan of the whole mute list every time
            .collect::<std::collections::HashSet<_>>()
    };

    let baseline = app
        .state::<Inbox>()
        .state()
        .baselines
        .get(entry_id)
        .cloned()
        .unwrap_or_default();

    let total = sharkord::unread_total(read_states, &baseline, &muted);

    if app.state::<Inbox>().set_unread(entry_id, total) {
        publish(app);
    }
}

/// How often Shiver re-reads the muted channels out of the server on screen.
///
/// The same cadence as the desktop client's own drain, for the same reason: it is the page that
/// knows, and Shiver has to have the answer before the user navigates rather than after.
const MUTE_POLL: Duration = Duration::from_secs(1);

/// Keeps the core's copy of the muted channels in step with the page that owns them.
///
/// Only the page can answer. It is where Shiver's list and the copy the companion plugin keeps on the
/// server are merged, and that merge is what carries a mute made on a desktop to this phone. The
/// core needs the result because the moment the user leaves a server it starts counting that
/// server's unread itself, and muted channels are what it has to leave out.
///
/// Idle unless a server is on screen, and `replace_mutes` does nothing when the list has not moved,
/// so the steady state is one small `eval` a second and no writes at all.
pub fn watch_mutes(app: &AppHandle) {
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(MUTE_POLL);

        loop {
            ticker.tick().await;

            let Some(entry_id) = handle.state::<webview::Showing>().server() else {
                // Shiver's own pages have no mute list to read
                continue;
            };

            webview::read_mutes(&handle, &entry_id);
        }
    });
}

/// Replaces one server's muted channels with what its own page reports, and recounts its badge.
///
/// The page is the authority rather than this store, because the page is where the two copies meet:
/// the bridge merges Shiver's local list with the one the companion plugin keeps on the server, which
/// is what carries a mute made on a desktop over to this phone. The core wants the result because
/// it counts that server's unread itself the moment the user leaves it.
///
/// Does nothing when the list has not moved, so leaving a server Shiver has already asked about does
/// not rewrite the store on every navigation.
pub fn replace_mutes(app: &AppHandle, entry_id: &str, channels: &[i64]) {
    let store = app.state::<Store>();

    let unchanged = {
        let registry = store.registry();
        let mut current = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == entry_id)
            .map(|muted| muted.channel_id)
            .collect::<Vec<_>>();

        let mut next = channels.to_vec();

        current.sort_unstable();
        next.sort_unstable();
        next.dedup();

        current == next
    };

    if unchanged {
        return;
    }

    let written = store.update(|registry| {
        registry.muted.retain(|muted| muted.entry_id != entry_id);

        for channel_id in channels {
            registry.muted.push(crate::model::MutedChannel {
                entry_id: entry_id.to_string(),
                channel_id: *channel_id,
            });
        }

        Ok(())
    });

    if let Err(error) = written {
        eprintln!("[shiver] could not store {entry_id}'s muted channels: {error}");

        return;
    }

    // the badge is wrong until this runs: the count standing in the rail still includes whatever
    // was just muted, and nothing else will recompute it until that server next says something
    let read_states = app
        .state::<Inbox>()
        .state()
        .read_states
        .get(entry_id)
        .cloned();

    if let Some(read_states) = read_states {
        recount(app, entry_id, &read_states);
    }
}

/// Tells both rails what the counts are now.
fn publish(app: &AppHandle) {
    let unread = app.state::<Inbox>().unread();

    // Shiver's own pages, which have IPC and can simply listen
    let _ = app.emit(INBOX_EVENT, &unread);

    push_to_page(app, &unread);
}

/// Updates the rail drawn inside a server's page.
///
/// That page cannot listen for a Tauri event — it is on the server's origin and has no IPC, which
/// is the rule that keeps a server from reaching anything Shiver knows. So the core calls *into* it,
/// the same direction the bridge is installed in. The counts are the only thing sent, and a count
/// is not an address.
fn push_to_page(app: &AppHandle, unread: &HashMap<String, u32>) {
    if app.state::<webview::Showing>().server().is_none() {
        return;
    }

    let Ok(window) = webview::main_window(app) else {
        return;
    };

    let Ok(payload) = serde_json::to_string(unread) else {
        return;
    };

    let _ = window.eval(format!(
        "window.__SHIVER_UNREAD__ && window.__SHIVER_UNREAD__({payload})"
    ));
}

/// Reads a token out of the page currently on screen, so Shiver can reach that server later.
///
/// Shiver holds no credentials on Android and cannot ask a server for a token itself, so borrowing
/// the one the user's own session already produced is the only way in. `eval_with_callback` rather
/// than the bridge: the bridge has no way to answer, and a token must not travel through a url
/// fragment where it would end up in history.
///
/// One evaluation, issued straight from the caller. It was briefly a retry loop on its own thread,
/// on the theory that the live session appears too late to catch — but that rebuilt the mechanism
/// around a guess and broke it, and none of the threads involved can report what they did (see the
/// note on `watch`). The timing worry is answered by `TOKEN_SCRIPT` instead: the auto-login token is
/// in `localStorage` from the moment the page loads, so there is nothing to wait for. A page that
/// genuinely has no token yet is picked up on its next load, which a Sharkord sign-in causes anyway.
///
/// Whether the token is *kept* is the user's, not Shiver's: a session found only in `sessionStorage`
/// belongs to someone who did not ask to stay signed in, and is used for this run and not stored.
/// See `TOKEN_SCRIPT` and `remember_session_for_this_run`.
pub fn harvest_token(app: &AppHandle, entry_id: String) {
    let Ok(window) = webview::main_window(app) else {
        return;
    };

    let handle = app.clone();

    let _ = window.eval_with_callback(TOKEN_SCRIPT, move |raw| {
        // arrives json-encoded, so no token at all is the literal `null`
        let Ok(serde_json::Value::String(answer)) = serde_json::from_str::<serde_json::Value>(&raw)
        else {
            return;
        };

        // Which storage it came from decides whether Shiver may keep it. `kept:` is Sharkord's
        // auto-login token, which the user asked for; `live:` is the session alone, which they did
        // not — see `TOKEN_SCRIPT`. An answer in neither shape is not a token Shiver understands.
        let Some((token, persist)) = answer
            .strip_prefix("kept:")
            .map(|token| (token, true))
            .or_else(|| answer.strip_prefix("live:").map(|token| (token, false)))
        else {
            return;
        };

        if token.is_empty() {
            return;
        }

        // Handed to a thread of its own, and this is not tidiness. Storing a session reaches back
        // into the platform — the write is a blocking round trip into the JVM, and the sync that
        // follows ends up calling `eval` — and doing either from inside an eval callback deadlocks
        // the app: this thread waits on the main thread while the main thread waits on the webview
        // machinery this callback is holding. It showed up as an ANR on the next touch, with the
        // store file already written, which reads like a hang unrelated to the write that caused it.
        let out = handle.clone();
        let id = entry_id.clone();
        let token = token.to_string();

        std::thread::spawn(move || {
            if persist {
                remember_session(&out, &id, &token);
            } else {
                remember_session_for_this_run(&out, &id, &token);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> sharkord::Joined {
        sharkord::Joined {
            own_user_id: Some(1),
            dm_channels: vec![9],
            channel_names: HashMap::from([(4, "general".to_string()), (9, "dm".to_string())]),
            user_names: HashMap::from([(2, "Smiddy".to_string())]),
            ..Default::default()
        }
    }

    fn entry(id: &str, name: &str, position: i32) -> crate::model::ServerEntry {
        crate::model::ServerEntry {
            accept_any_size: false,
            id: id.into(),
            origin: format!("https://{id}.example.com"),
            server_id: None,
            name: name.into(),
            icon_url: None,
            icon_data: None,
            identity: None,
            account_label: None,
            folder_id: None,
            position,
        }
    }

    /// A server-supplied name is bounded before it reaches the shade, and bounded by *characters*:
    /// the same cut by bytes would panic in the middle of one of these.
    #[test]
    fn a_very_long_name_is_shortened_without_splitting_a_character() {
        let shouting = "é".repeat(5_000);
        let line = notice(
            &server(),
            &[],
            &sharkord::NewMessage {
                channel_id: 4,
                user_id: None,
                plugin_id: Some(shouting),
                text: "hello".into(),
            },
        )
        .expect("a message from a plugin is still worth a notice");

        assert!(line.starts_with(&"é".repeat(MAX_AUTHOR)));
        assert!(line.contains("in #general: hello"));
        // the name, the ellipsis, and " in #general: hello"
        assert_eq!(line.chars().count(), MAX_AUTHOR + 1 + 19);
    }

    /// And so is the message itself, which is a preview rather than the message.
    #[test]
    fn a_very_long_message_is_shortened() {
        let line = notice(&server(), &[], &from(2, 4, &"a".repeat(10_000)))
            .expect("a long message is still a message");

        assert!(line.ends_with('…'));
        assert_eq!(
            line.chars().count(),
            "Smiddy in #general: ".len() + MAX_BODY + 1
        );
    }

    fn conversation(channel_id: i64, name: &str, at: Option<u64>) -> sharkord::DirectMessage {
        sharkord::DirectMessage {
            channel_id,
            user_name: name.into(),
            last_message_at: at,
        }
    }

    /// Newest first, and **across** servers rather than grouped by them. Grouping by rail order was
    /// what made the list reshuffle whenever the user switched servers.
    #[test]
    fn conversations_are_ordered_by_the_latest_message() {
        let servers = vec![entry("a", "Alpha", 7), entry("b", "Beta", 2)];
        let dms = HashMap::from([
            (
                "a".to_string(),
                vec![
                    conversation(1, "Robin", Some(3_000)),
                    conversation(2, "Wren", Some(1_000)),
                ],
            ),
            ("b".to_string(), vec![conversation(3, "Sam", Some(2_000))]),
        ]);

        let collected = collect_dms(&servers, &dms);
        let names: Vec<&str> = collected.iter().map(|dm| dm.user_name.as_str()).collect();

        // Beta sits above Alpha in the rail, and its conversation still lands in the middle
        assert_eq!(names, vec!["Robin", "Sam", "Wren"]);
    }

    /// The order does not depend on which server is on screen, which was the whole complaint: the
    /// same input in a different rail order is the same list.
    #[test]
    fn the_order_does_not_depend_on_the_rail() {
        let dms = HashMap::from([
            ("a".to_string(), vec![conversation(1, "Robin", Some(3_000))]),
            ("b".to_string(), vec![conversation(3, "Sam", Some(2_000))]),
        ]);

        let one = collect_dms(&[entry("a", "Alpha", 0), entry("b", "Beta", 1)], &dms);
        let other = collect_dms(&[entry("b", "Beta", 0), entry("a", "Alpha", 1)], &dms);

        assert_eq!(
            one.iter().map(|dm| dm.channel_id).collect::<Vec<_>>(),
            other.iter().map(|dm| dm.channel_id).collect::<Vec<_>>()
        );
    }

    /// A server that answered without the field sorts last: unknown is not recent, and putting it
    /// first would give the least-known rows the most prominent place.
    #[test]
    fn a_conversation_with_no_timestamp_sorts_last() {
        let dms = HashMap::from([(
            "a".to_string(),
            vec![
                conversation(1, "Robin", None),
                conversation(2, "Sam", Some(1_000)),
            ],
        )]);

        let collected = collect_dms(&[entry("a", "Alpha", 0)], &dms);

        assert_eq!(collected[0].user_name, "Sam");
        assert_eq!(collected[1].user_name, "Robin");
    }

    /// Ties break on the name, so two conversations sharing a timestamp cannot swap places between
    /// reads and make the list look unstable again.
    #[test]
    fn a_tie_breaks_on_the_name() {
        let dms = HashMap::from([(
            "a".to_string(),
            vec![
                conversation(1, "Wren", Some(1_000)),
                conversation(2, "Robin", Some(1_000)),
            ],
        )]);

        let collected = collect_dms(&[entry("a", "Alpha", 0)], &dms);

        assert_eq!(collected[0].user_name, "Robin");
        assert_eq!(collected[1].user_name, "Wren");
    }

    /// A server Shiver has never reached contributes nothing, rather than an entry with no
    /// conversations behind it.
    #[test]
    fn a_server_never_reached_contributes_nothing() {
        assert!(collect_dms(&[entry("a", "Alpha", 0)], &HashMap::new()).is_empty());
    }

    fn from(user_id: i64, channel_id: i64, text: &str) -> sharkord::NewMessage {
        sharkord::NewMessage {
            channel_id,
            user_id: Some(user_id),
            plugin_id: None,
            text: text.to_string(),
        }
    }

    /// What the shade says, and it has to say who and where — a line that only says a server has
    /// something new is what the badge already says.
    #[test]
    fn a_message_reads_as_who_said_what_and_where() {
        assert_eq!(
            notice(&server(), &[], &from(2, 4, "are you around?")),
            Some("Smiddy in #general: are you around?".into())
        );
    }

    /// A conversation has no channel worth naming: it is the person.
    #[test]
    fn a_direct_message_is_named_by_the_person() {
        assert_eq!(
            notice(&server(), &[], &from(2, 9, "hello")),
            Some("Smiddy: hello".into())
        );
    }

    /// The two silences: a message this user wrote, and a channel they muted. Muting is Shiver's own
    /// and the server keeps sending, so it has to be applied here or a muted channel still buzzes.
    #[test]
    fn nothing_is_said_about_your_own_messages_or_a_muted_channel() {
        assert_eq!(notice(&server(), &[], &from(1, 4, "mine")), None);
        assert_eq!(notice(&server(), &[4], &from(2, 4, "theirs")), None);
    }

    /// Everything else still has to produce a line: a file with no words, a plugin rather than a
    /// person, and someone who joined after Shiver connected and so has no name in the list.
    #[test]
    fn a_line_survives_a_missing_name_a_missing_channel_and_no_words() {
        assert_eq!(
            notice(&server(), &[], &from(2, 4, "")),
            Some("Smiddy in #general: Sent an attachment".into())
        );
        assert_eq!(
            notice(&server(), &[], &from(77, 4, "hi")),
            Some("Someone in #general: hi".into())
        );
        assert_eq!(
            notice(&server(), &[], &from(2, 88, "hi")),
            Some("Smiddy in #a channel: hi".into())
        );

        let from_plugin = sharkord::NewMessage {
            channel_id: 4,
            user_id: None,
            plugin_id: Some("shiver".into()),
            text: "built".into(),
        };

        assert_eq!(
            notice(&server(), &[], &from_plugin),
            Some("shiver in #general: built".into())
        );
    }

    /// Both kinds of key have to lead back to the entry, or `restore` reads a carried value as a
    /// server that no longer exists and deletes it — quietly, on every launch.
    #[test]
    fn a_store_key_names_the_entry_it_belongs_to() {
        assert_eq!(entry_of("abc"), "abc");
        assert_eq!(entry_of(&carried_store_key("abc")), "abc");
    }

    #[test]
    fn a_token_is_only_news_the_first_time() {
        let inbox = Inbox::default();

        assert!(inbox.remember_token("a", "one"));
        assert!(!inbox.remember_token("a", "one"));
        assert!(inbox.remember_token("a", "two"));
    }

    /// A count that has not moved must not wake both rails, or every message in a busy channel
    /// would repaint them.
    #[test]
    fn an_unchanged_count_is_not_a_change() {
        let inbox = Inbox::default();

        assert!(inbox.set_unread("a", 3));
        assert!(!inbox.set_unread("a", 3));
        assert!(inbox.set_unread("a", 4));
    }

    #[test]
    fn forgetting_a_server_drops_its_token_and_its_badge() {
        let inbox = Inbox::default();

        inbox.remember_token("a", "one");
        inbox.set_unread("a", 2);
        inbox.forget("a");

        assert!(inbox.unread().is_empty());
        assert!(inbox.remember_token("a", "one"), "the token was kept");
    }
}
