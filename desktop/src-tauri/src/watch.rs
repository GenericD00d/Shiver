//! The core's own socket to every server that has no page open (see `sharkord-client`).
//!
//! A page costs ~100 MB; a socket almost nothing. Sockets report messages, conversations and read
//! state; the server on screen still has its page, which alone knows fullscreen, voice and the
//! channel being viewed.
//!
//! The rail badge is the feed's unread count (what arrived while Shiver ran) plus `Missed` (what
//! arrived while it was closed, counted once per server per run against the stored floor, and
//! afterwards only ever lowered as channels are read).

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use sharkord_client as sharkord;
use shiver_core::LockExt;
use tauri::{async_runtime::JoinHandle, AppHandle, Manager, Runtime};

use crate::{
    commands, drain,
    drain::Readiness,
    feed::{DmChannel, Feed, RawNotification},
    model::ServerEntry,
    secrets::{self, Secret},
    store::{RegistryStore, Store},
    webviews,
};

/// What each server held unread above its floor when Shiver connected, per entry.
#[derive(Default)]
pub struct Missed(Mutex<HashMap<String, usize>>);

impl Missed {
    pub fn counts(&self) -> HashMap<String, usize> {
        self.0.locked().clone()
    }

    pub fn total(&self) -> usize {
        self.0.locked().values().sum()
    }

    fn set(&self, entry_id: &str, count: usize) {
        let mut held = self.0.locked();

        if count == 0 {
            held.remove(entry_id);
        } else {
            held.insert(entry_id.to_string(), count);
        }
    }

    /// Lowers (never raises) an entry's count. Returns whether it changed.
    fn lower_to(&self, entry_id: &str, count: usize) -> bool {
        let mut held = self.0.locked();

        match held.get(entry_id).copied() {
            Some(existing) if count < existing => {
                if count == 0 {
                    held.remove(entry_id);
                } else {
                    held.insert(entry_id.to_string(), count);
                }

                true
            }
            _ => false,
        }
    }

    fn clear(&self, entry_id: &str) -> bool {
        self.0.locked().remove(entry_id).is_some()
    }
}

/// Each watched server's live per-channel unread, which later read events are applied to.
#[derive(Default)]
pub struct ReadStates(Mutex<HashMap<String, HashMap<i64, u32>>>);

impl ReadStates {
    fn apply<T>(&self, entry_id: &str, change: impl FnOnce(&mut HashMap<i64, u32>) -> T) -> T {
        change(self.0.locked().entry(entry_id.to_string()).or_default())
    }

    fn knows(&self, entry_id: &str) -> bool {
        self.0.locked().contains_key(entry_id)
    }
}

/// Which servers have the companion plugin: absent = not connected yet, `None` = not installed.
#[derive(Default)]
pub struct Plugins(Mutex<HashMap<String, Option<String>>>);

impl Plugins {
    pub fn all(&self) -> HashMap<String, Option<String>> {
        self.0.locked().clone()
    }
}

/// Servers that have been reported as sending too-large messages this run.
#[derive(Default)]
pub struct Reported(Mutex<HashSet<String>>);

impl Reported {
    pub fn mentioned(&self, entry_id: &str) -> bool {
        self.0.locked().contains(entry_id)
    }
}

/// The running socket tasks, with an id per task so a finishing task never removes its replacement,
/// and the entries parked because they have no usable session (until the user signs in).
#[derive(Default)]
pub struct Watcher {
    tasks: Mutex<HashMap<String, (u64, JoinHandle<()>)>>,
    parked: Mutex<HashSet<String>>,
    next_id: AtomicU64,
}

impl Watcher {
    pub fn is_watching(&self, entry_id: &str) -> bool {
        self.tasks.locked().contains_key(entry_id)
    }
}

/// Brings the live sockets in line with the rail: one per signed-in server with no page open.
/// Idempotent, so callers just say "something changed".
pub fn sync(app: &AppHandle) {
    let watcher = app.state::<Watcher>();
    let mut tasks = watcher.tasks.locked();
    let parked = watcher.parked.locked().clone();

    let wanted: Vec<ServerEntry> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .filter(|entry| entry.identity.is_some() && !parked.contains(&entry.id))
        .filter(|entry| {
            app.get_webview(&webviews::webview_label(&entry.id))
                .is_none()
        })
        .cloned()
        .collect();

    tasks.retain(|entry_id, (_, task)| {
        let keep = wanted.iter().any(|entry| &entry.id == entry_id);

        if !keep {
            task.abort();
            app.state::<Readiness>().forget_entry(entry_id);
        }

        keep
    });

    for entry in wanted {
        if tasks.contains_key(&entry.id) {
            continue;
        }

        let id = watcher.next_id.fetch_add(1, Ordering::Relaxed);
        let handle = app.clone();
        let entry_id = entry.id.clone();

        tasks.insert(
            entry_id,
            (id, tauri::async_runtime::spawn(watch(handle, entry, id))),
        );
    }
}

/// Drops a server's socket (and any park) so the next attempt uses whatever just changed.
pub fn restart(app: &AppHandle, entry_id: &str) {
    let watcher = app.state::<Watcher>();

    if let Some((_, task)) = watcher.tasks.locked().remove(entry_id) {
        task.abort();
    }

    watcher.parked.locked().remove(entry_id);
    app.state::<Reported>().0.locked().remove(entry_id);

    sync(app);
}

/// Removes this task's own record, and parks the entry if it stopped for want of a session.
fn finished(app: &AppHandle, entry_id: &str, task_id: u64, park: bool) {
    let watcher = app.state::<Watcher>();
    let mut tasks = watcher.tasks.locked();

    if tasks.get(entry_id).is_some_and(|(id, _)| *id == task_id) {
        tasks.remove(entry_id);
        app.state::<Readiness>().forget_entry(entry_id);

        if park {
            watcher.parked.locked().insert(entry_id.to_string());
        }
    }
}

/// Holds one server's connection open, reconnecting with backoff.
async fn watch(app: AppHandle, entry: ServerEntry, task_id: u64) {
    let origin = entry.origin.clone();
    let mut failures: u32 = 0;
    let mut refusals: u32 = 0;

    loop {
        // asked each attempt, which is what renews an expiring session from the stored password
        let Some(token) = commands::ensure_session(&entry).await else {
            eprintln!("[shiver] no session for {origin}; it is not watched until it is signed in");

            return finished(&app, &entry.id, task_id, true);
        };

        let trusted = app
            .state::<Store>()
            .registry()
            .server(&entry.id)
            .is_some_and(|server| server.accept_any_size);

        match sharkord::open(&origin, &token, trusted).await {
            Ok(mut session) => {
                failures = 0;
                refusals = 0;
                on_joined(&app, &entry.id, &session.joined);

                while let Some(event) = session.next_event().await {
                    on_event(&app, &entry.id, &session.joined, event);
                }

                eprintln!("[shiver] {origin} closed the connection");
            }
            Err(sharkord::Error::Refused(reason)) => {
                // drop the refused token so `ensure_session` signs in again next time round
                eprintln!("[shiver] {origin} refused Shiver's session ({reason})");
                let _ = secrets::forget_off_thread(Secret::Session, &entry.id).await;

                // a server that refuses even fresh sessions is left alone until the user signs in
                refusals += 1;

                if refusals >= 3 {
                    return finished(&app, &entry.id, task_id, true);
                }
            }
            Err(sharkord::Error::TooLarge { size, max }) => {
                eprintln!(
                    "[shiver] {origin} sent {size} bytes in one message; Shiver accepts {max}"
                );
                report_too_large(&app, &entry.id, size);
            }
            Err(error) => eprintln!("[shiver] could not watch {origin}: {error}"),
        }

        app.state::<Readiness>().disconnected(&entry.id);

        let wait = sharkord::retry_delay(failures, &entry.id);

        failures = failures.saturating_add(1);
        tokio::time::sleep(wait).await;
    }
}

fn on_joined(app: &AppHandle, entry_id: &str, joined: &sharkord::Joined) {
    app.state::<Readiness>().set(entry_id, true);
    publish_dms(app, entry_id, joined);

    // counted only on the first join of the run; a reconnect re-reads messages already in the feed
    if !app.state::<ReadStates>().knows(entry_id) {
        count_missed(
            app,
            entry_id,
            &joined.read_states,
            joined.shared_floor.clone(),
        );
    }

    app.state::<Plugins>()
        .0
        .locked()
        .insert(entry_id.to_string(), joined.plugin_version.clone());
    app.state::<ReadStates>()
        .0
        .locked()
        .insert(entry_id.to_string(), joined.read_states.clone());

    drain::notify_feed_changed(app);
}

fn on_event(app: &AppHandle, entry_id: &str, joined: &sharkord::Joined, event: sharkord::Event) {
    match event {
        sharkord::Event::Posted(message) => announce(app, entry_id, joined, &message),
        // a new message: already in the feed, so only tracked for later reads
        sharkord::Event::Unread { channel_id, delta } => {
            app.state::<ReadStates>().apply(entry_id, |states| {
                sharkord::apply_delta(states, channel_id, delta)
            });
        }
        // read somewhere (this machine, the phone, a browser)
        sharkord::Event::UnreadSet { channel_id, count } => {
            app.state::<ReadStates>().apply(entry_id, |states| {
                sharkord::set_unread(states, channel_id, count)
            });

            if count == 0 {
                crate::badges::channel_viewed(app, entry_id, channel_id);
            } else if recount(app, entry_id) {
                drain::notify_feed_changed(app);
            }
        }
    }
}

/// Muted channel ids for one entry, as a set.
fn muted_set<R: Runtime>(app: &AppHandle<R>, entry_id: &str) -> HashSet<i64> {
    app.state::<Store>()
        .registry()
        .muted_for(entry_id)
        .into_iter()
        .collect()
}

/// Recomputes `Missed` for an entry from its live read states, lowering it if reads brought it down.
fn recount<R: Runtime>(app: &AppHandle<R>, entry_id: &str) -> bool {
    let floor = app
        .state::<Store>()
        .registry()
        .baselines
        .get(entry_id)
        .cloned()
        .unwrap_or_default();
    let muted = muted_set(app, entry_id);
    let recomputed = app
        .state::<ReadStates>()
        .apply(entry_id, |states| missed_since(states, &floor, &muted));

    app.state::<Missed>().lower_to(entry_id, recomputed)
}

/// A channel the user is looking at (or read elsewhere) is read: clear it and lower the badge.
pub fn channel_read<R: Runtime>(app: &AppHandle<R>, entry_id: &str, channel_id: i64) {
    app.state::<ReadStates>()
        .apply(entry_id, |states| states.remove(&channel_id));

    if recount(app, entry_id) {
        drain::notify_feed_changed(app);
    }
}

/// "Mark all as read" from the rail menu.
pub fn mark_read(app: &AppHandle, entry_id: &str) {
    if app.state::<Missed>().clear(entry_id) {
        drain::notify_feed_changed(app);
    }
}

/// Hands a server's page its floor, so the companion plugin can share it with the user's other
/// devices. The floor itself does not move.
pub fn publish_floor(app: &AppHandle, entry_id: &str) {
    let Some(floor) = app
        .state::<Store>()
        .registry()
        .baselines
        .get(entry_id)
        .cloned()
    else {
        return;
    };

    let Some(webview) = app.get_webview(&webviews::webview_label(entry_id)) else {
        return;
    };

    let floor: HashMap<String, u32> = floor
        .into_iter()
        .map(|(channel, count)| (channel.to_string(), count))
        .collect();

    if let Ok(payload) = serde_json::to_string(&floor) {
        let _ = webview.eval(format!(
            "window.__SHIVER_SET_READ_FLOOR__ && window.__SHIVER_SET_READ_FLOOR__({payload})"
        ));
    }
}

/// On the first join of a run: the missed count against the floor (the plugin's shared floor wins
/// over the local one). With no floor at all, this join becomes the floor and nothing is counted, so
/// a newly added server's backlog is not announced as unread.
fn count_missed<R: Runtime>(
    app: &AppHandle<R>,
    entry_id: &str,
    read_states: &HashMap<i64, u32>,
    shared: Option<HashMap<i64, u32>>,
) {
    let store = app.state::<Store>();
    let local = store.registry().baselines.get(entry_id).cloned();

    let Some(baseline) = shared.or(local) else {
        let states = read_states.clone();
        let _ = store.update(|registry| {
            registry.baselines.insert(entry_id.to_string(), states);

            Ok(())
        });

        return;
    };

    app.state::<Missed>().set(
        entry_id,
        missed_since(read_states, &baseline, &muted_set(app, entry_id)),
    );
}

/// Unread above the floor, skipping muted channels; a channel below its floor counts zero.
fn missed_since(
    read_states: &HashMap<i64, u32>,
    baseline: &HashMap<i64, u32>,
    muted: &HashSet<i64>,
) -> usize {
    sharkord::unread_total(read_states, baseline, muted) as usize
}

fn account_label(entry: &ServerEntry) -> String {
    entry
        .account_label
        .clone()
        .or_else(|| entry.identity.clone())
        .unwrap_or_default()
}

/// Hands the join's conversation list to the inbox.
fn publish_dms(app: &AppHandle, entry_id: &str, joined: &sharkord::Joined) {
    let Some((server_name, label)) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .map(|entry| (entry.name.clone(), account_label(entry)))
    else {
        return;
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
        .set_dms(entry_id, &server_name, &label, None, channels);
}

/// Files one message in the feed, unless it is the user's own or a page is already reporting.
fn announce(
    app: &AppHandle,
    entry_id: &str,
    joined: &sharkord::Joined,
    message: &sharkord::NewMessage,
) {
    if message.user_id.is_some() && message.user_id == joined.own_user_id {
        return;
    }

    if app
        .get_webview(&webviews::webview_label(entry_id))
        .is_some()
    {
        return;
    }

    let Some(server_name) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .map(|entry| entry.name.clone())
    else {
        return;
    };

    let author = message
        .plugin_id
        .clone()
        .or_else(|| {
            message
                .user_id
                .and_then(|id| joined.user_names.get(&id).cloned())
        })
        .unwrap_or_else(|| "Someone".into());

    let raw = RawNotification {
        channel_id: Some(message.channel_id),
        channel_name: joined.channel_names.get(&message.channel_id).cloned(),
        author,
        body: if message.text.is_empty() {
            "Sent an attachment".into()
        } else {
            message.text.clone()
        },
        icon_url: None,
        is_dm: joined.dm_channels.contains(&message.channel_id),
    };

    let muted = muted_set(app, entry_id).contains(&message.channel_id);

    if app
        .state::<Feed>()
        .push(entry_id, &server_name, None, raw, muted)
    {
        drain::notify_feed_changed(app);
    }
}

/// Says once per server per run that it is sending more than Shiver accepts, since that failure
/// will not fix itself.
fn report_too_large(app: &AppHandle, entry_id: &str, size: usize) {
    let Some(name) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .map(|entry| entry.name.clone())
    else {
        return;
    };

    if !app
        .state::<Reported>()
        .0
        .locked()
        .insert(entry_id.to_string())
    {
        return;
    }

    let size = match size as f64 / (1024.0 * 1024.0) {
        mb if mb < 1.0 => format!("{} KB", size / 1024),
        mb => format!("{mb:.1} MB"),
    };

    app.state::<Feed>().push(
        entry_id,
        &name,
        None,
        RawNotification {
            channel_id: None,
            channel_name: None,
            author: "Shiver".into(),
            body: format!(
                "{name} sent more in one message than Shiver accepts ({size}), so it is not being watched. \
                 Allow larger messages from it in the rail's menu if you trust it."
            ),
            icon_url: None,
            is_dm: false,
        },
        false,
    );

    drain::notify_feed_changed(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn muted<const N: usize>(channels: [i64; N]) -> HashSet<i64> {
        channels.into_iter().collect()
    }

    #[test]
    fn missed_counts_only_what_is_above_the_floor_and_unmuted() {
        let floor = HashMap::from([(1, 5), (2, 9)]);

        assert_eq!(
            missed_since(&HashMap::from([(1, 8), (2, 9)]), &floor, &muted([])),
            3
        );
        assert_eq!(
            missed_since(&HashMap::from([(1, 2)]), &floor, &muted([])),
            0
        );
        assert_eq!(
            missed_since(&HashMap::from([(7, 4)]), &floor, &muted([])),
            4
        );
        assert_eq!(
            missed_since(&HashMap::from([(7, 4), (1, 9)]), &floor, &muted([7])),
            4
        );
    }

    /// The reported bug: a read elsewhere must bring the badge down, and nothing may push it up.
    #[test]
    fn missed_is_only_ever_lowered() {
        let missed = Missed::default();

        missed.set("a", 5);
        assert!(!missed.lower_to("a", 7));
        assert!(missed.lower_to("a", 2));
        assert_eq!(missed.counts().get("a"), Some(&2));
        assert!(missed.lower_to("a", 0));
        assert!(missed.counts().is_empty());
        assert!(!missed.lower_to("never-seen", 0));
    }
}
