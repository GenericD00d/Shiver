//! The unified notification and DM feed.
//!
//! Every live server webview queues events for Shiver inside the page; the core drains those queues
//! on a timer with `eval_with_callback` and merges them here. Nothing is pushed *from* a page: a
//! Sharkord page has no Tauri IPC at all, so this is the only way data crosses out of an origin,
//! and it crosses into the core rather than into another server.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::voice::VoiceSnapshot;

/// Enough history to scroll through, small enough to keep in memory and to serialise cheaply.
const MAX_ENTRIES: usize = 300;

/// How long a notification's fields may be, in characters.
///
/// `MAX_ENTRIES` caps how *many* notifications Shiver keeps and said nothing about how large one may
/// be, so a hostile server could push a single ten-megabyte "message" — three hundred of which Shiver
/// would hold in memory and serialise to the feed on every change — or a five-thousand-character
/// username, which wrecks the list it is drawn in. A name is a name and a preview is a preview:
/// neither needs more than this, and the feed is a preview of a message rather than the message.
const MAX_AUTHOR: usize = 100;
const MAX_BODY: usize = 500;
const MAX_CHANNEL_NAME: usize = 100;

/// How close together two identical-looking notifications must be to count as one message.
///
/// **Deliberately short.** There is no id in common to match on — the two copies arrive by
/// different routes, one off the socket and one out of the page, and only one of those carries a
/// message id at all. So the match is on who said what, and that is also true of a person sending
/// the same word twice. Thirty seconds covers the race that causes this (a page connecting while
/// its server's socket is still up, both announcing what arrives in between) and leaves somebody
/// repeating themselves a minute later with the two lines they deserve.
const DUPLICATE_WINDOW_MS: u64 = 30 * 1000;
/// A url is longer than a name but not unbounded; this is past every real one.
const MAX_URL: usize = 2048;

/// How many conversations one server may contribute to the unified list.
const MAX_DMS: usize = 500;

/// Shortens a string from a server to `limit` characters.
///
/// By characters and not bytes, deliberately: `String` slicing is by byte offset and panics on a
/// boundary inside a codepoint, so `&text[..limit]` on anything non-ascii is a crash a server could
/// choose to cause. Returns the original untouched when it is already short enough, which is every
/// real message.
fn clamp(text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }

    // an ellipsis rather than a bare cut, so a truncated preview reads as truncated
    text.chars().take(limit).collect::<String>() + "…"
}

/// Milliseconds as they arrive from a page.
///
/// A webview hands its numbers back as doubles, and serde_json writes a large one in exponential
/// form — `1.769871194e+12` — which `u64` refuses outright. Timestamps are the only values Shiver
/// takes from a page that are big enough to hit this.
///
/// It has to be tolerated rather than avoided, because the failure is not confined to the field:
/// one unparseable number fails the whole `DrainResult`, the tick is dropped, and notifications,
/// DMs and mutes go with it. That is exactly what happened once the message cache started sending a
/// timestamp, and it is why anything crossing this boundary accepts both forms.
pub fn optional_millis<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Millis {
        Int(u64),
        Float(f64),
    }

    Ok(
        Option::<Millis>::deserialize(deserializer)?.and_then(|value| match value {
            Millis::Int(millis) => Some(millis),
            // a negative or non-finite timestamp is not a time, and is dropped rather than wrapped
            Millis::Float(millis) if millis.is_finite() && millis >= 0.0 => {
                Some(millis.round() as u64)
            }
            Millis::Float(_) => None,
        }),
    )
}

/// One item in the bell feed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: u64,
    pub entry_id: String,
    pub server_name: String,
    pub channel_id: Option<i64>,
    pub channel_name: Option<String>,
    pub author: String,
    pub body: String,
    pub icon_url: Option<String>,
    pub is_dm: bool,
    pub at: u64,
    pub read: bool,
    /// The version on offer, when this entry is Shiver rather than a server.
    ///
    /// A separate field rather than a magic `entry_id`, because the popup has to draw this one
    /// differently — with a button — and "is this a message" should not be a guess about a string.
    #[serde(default)]
    pub update: Option<String>,
}

/// A direct message conversation, as reported by one server's page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmChannel {
    pub channel_id: i64,
    pub name: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    /// when Shiver last saw a message here. null until one arrives while Shiver is running: the plugin
    /// store exposes channels but not their messages, so there is no history to read on connect.
    #[serde(default, deserialize_with = "optional_millis")]
    pub last_message_at: Option<u64>,
}

/// A DM conversation plus which rail entry (and therefore which account) it belongs to, which is
/// what lets the inbox say who the user is messaging *as*.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmEntry {
    pub entry_id: String,
    pub server_name: String,
    pub account_label: String,
    pub channel: DmChannel,
}

/// What a page hands back when drained.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainResult {
    #[serde(default)]
    pub notifications: Vec<RawNotification>,
    /// absent when the page has nothing new to say about its DM list
    #[serde(default)]
    pub dms: Option<Vec<DmChannel>>,
    /// mutes toggled from Sharkord's own channel context menu
    #[serde(default)]
    pub mutes: Vec<QueuedMute>,
    /// the Shiver plugin's reconciled mute list, sent once per connect when the plugin is installed
    #[serde(default)]
    pub synced_mutes: Option<Vec<i64>>,
    /// a conversation Shiver asked the page to open and it could not find
    #[serde(default)]
    pub open_dm_failed: Option<String>,
    /// addresses the page tried to open in a new window, which is Shiver's to hand to the browser
    #[serde(default)]
    pub open: Vec<String>,
    /// the client has connected and is showing the server, so Shiver can stop covering it
    #[serde(default)]
    pub ready: bool,
    /// the client gave up on the seeded session and is asking for credentials
    #[serde(default)]
    pub signed_out: bool,
    /// something in the page is filling the screen, so Shiver's own chrome must get out of the way
    #[serde(default)]
    pub fullscreen: bool,
    /// the user's voice session on this server, absent when they are not in one
    #[serde(default)]
    pub voice: Option<VoiceSnapshot>,
    /// the channel this page is showing, whether or not its messages changed
    #[serde(default)]
    pub viewing_channel_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMute {
    pub channel_id: i64,
    pub muted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawNotification {
    #[serde(default)]
    pub channel_id: Option<i64>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub is_dm: bool,
}

#[derive(Default)]
struct FeedState {
    notifications: VecDeque<Notification>,
    dms: Vec<DmEntry>,
    next_id: u64,
}

#[derive(Default)]
pub struct Feed(Mutex<FeedState>);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

impl Feed {
    fn state(&self) -> std::sync::MutexGuard<'_, FeedState> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Adds one notification. Returns false when it was dropped because the channel is muted.
    pub fn push(
        &self,
        entry_id: &str,
        server_name: &str,
        raw: RawNotification,
        muted: bool,
    ) -> bool {
        if muted {
            return false;
        }

        let mut state = self.state();

        let author = clamp(raw.author, MAX_AUTHOR);
        let body = clamp(raw.body, MAX_BODY);

        // **The same message can be announced twice**, and it was. A server Shiver is watching over
        // a socket announces what arrives there; opening that server builds a page, and the page
        // announces what Sharkord's own client raises — which, for something still unread, is the
        // very same message. The badge went from 1 to 2 by being clicked on.
        //
        // Worse than the number: the two copies are not equally clearable. The socket's carries the
        // channel id, so reading that channel clears it; the page resolves a direct message's
        // channel by looking the author up in the store, which is empty for the first moments after
        // it connects, so its copy has no channel and `mark_channel_read` will not touch it. One
        // unread entry that nothing could ever clear.
        //
        // Matched on who said what rather than on an id, because there is no id in common: these
        // arrive by different routes from different halves of the same server. The window keeps a
        // person who really does send the same word twice from having the second one swallowed.
        if let Some(existing) = state.notifications.iter_mut().find(|entry| {
            !entry.read
                && entry.entry_id == entry_id
                && entry.author == author
                && entry.body == body
                && now_ms().saturating_sub(entry.at) < DUPLICATE_WINDOW_MS
        }) {
            // Keep whichever copy knows its channel, so the survivor is the clearable one. The
            // socket's copy usually arrives first and has it; this is for when the order is
            // reversed.
            if existing.channel_id.is_none() {
                existing.channel_id = raw.channel_id;
                existing.channel_name = raw.channel_name.map(|name| clamp(name, MAX_CHANNEL_NAME));
            }

            return false;
        }

        let id = state.next_id;

        state.next_id += 1;

        // Everything below this line came from a server and is bounded on the way in. Doing it here
        // rather than at the edge covers every route into the feed with one check.
        state.notifications.push_front(Notification {
            id,
            entry_id: entry_id.to_string(),
            server_name: server_name.to_string(),
            channel_id: raw.channel_id,
            channel_name: raw.channel_name.map(|name| clamp(name, MAX_CHANNEL_NAME)),
            author,
            body,
            icon_url: raw.icon_url.map(|url| clamp(url, MAX_URL)),
            is_dm: raw.is_dm,
            at: now_ms(),
            read: false,
            update: None,
        });

        state.notifications.truncate(MAX_ENTRIES);

        true
    }

    /// Offers a new version, replacing any offer already in the list.
    ///
    /// Replaced rather than added: the check runs every few hours for as long as Shiver is open,
    /// and a feed that collected one of these per check would bury the messages it exists for.
    pub fn push_update(&self, version: &str) {
        let mut state = self.state();

        state.notifications.retain(|entry| entry.update.is_none());

        let id = state.next_id;

        state.next_id += 1;

        state.notifications.push_front(Notification {
            id,
            entry_id: String::new(),
            server_name: "Shiver".into(),
            channel_id: None,
            channel_name: None,
            author: format!("Shiver {version}"),
            body: "A new version is available.".into(),
            icon_url: None,
            is_dm: false,
            at: now_ms(),
            read: false,
            update: Some(clamp(version.to_string(), MAX_AUTHOR)),
        });

        state.notifications.truncate(MAX_ENTRIES);
    }

    /// Replaces everything known about one entry's DM list. Called with that page's full list, so
    /// a conversation that disappeared server-side disappears here too.
    pub fn set_dms(
        &self,
        entry_id: &str,
        server_name: &str,
        account_label: &str,
        channels: Vec<DmChannel>,
    ) {
        let mut state = self.state();

        state.dms.retain(|dm| dm.entry_id != entry_id);

        // bounded on the way in for the same reason notifications are: the name is a name, and it
        // arrives from a page. The list itself is capped too — a page reporting thousands of
        // conversations is not a list anyone can use, and every one of them is held and serialised.
        for channel in channels.into_iter().take(MAX_DMS) {
            state.dms.push(DmEntry {
                entry_id: entry_id.to_string(),
                server_name: server_name.to_string(),
                account_label: account_label.to_string(),
                channel: DmChannel {
                    name: clamp(channel.name, MAX_AUTHOR),
                    icon_url: channel.icon_url.map(|url| clamp(url, MAX_URL)),
                    ..channel
                },
            });
        }
    }

    pub fn notifications(&self) -> Vec<Notification> {
        self.state().notifications.iter().cloned().collect()
    }

    pub fn dms(&self) -> Vec<DmEntry> {
        self.state().dms.clone()
    }

    pub fn unread_count(&self) -> usize {
        self.state()
            .notifications
            .iter()
            .filter(|entry| !entry.read)
            .count()
    }

    /// Unread per rail entry, for the badges on the server icons.
    ///
    /// Half of that badge — what arrived while Shiver was *running*, so every one of these has a
    /// message behind it. The other half is `watch::Missed`, which is what arrived while it was
    /// closed. Servers with nothing unread are absent rather than zero, so the rail can default.
    pub fn unread_by_entry(&self) -> HashMap<String, usize> {
        let mut counts: HashMap<String, usize> = HashMap::new();

        for entry in self
            .state()
            .notifications
            .iter()
            .filter(|entry| !entry.read)
        {
            *counts.entry(entry.entry_id.clone()).or_default() += 1;
        }

        counts
    }

    /// Marks one channel's notifications read, for a channel the user is looking at.
    ///
    /// Notifications with no channel are left alone: an unresolved channel could be any of them,
    /// and clearing those on the strength of a guess would drop something unread.
    pub fn mark_channel_read(&self, entry_id: &str, channel_id: i64) -> bool {
        let mut changed = false;

        for entry in self.state().notifications.iter_mut() {
            if entry.entry_id == entry_id && entry.channel_id == Some(channel_id) && !entry.read {
                entry.read = true;
                changed = true;
            }
        }

        changed
    }

    /// Marks one server's notifications read. Returns whether anything actually changed.
    ///
    /// This is what opening a server means: the badge on its icon is a claim that there is
    /// something there the user has not seen, and going to look at it settles that claim.
    pub fn mark_entry_read(&self, entry_id: &str) -> bool {
        let mut changed = false;

        for entry in self.state().notifications.iter_mut() {
            if entry.entry_id == entry_id && !entry.read {
                entry.read = true;
                changed = true;
            }
        }

        changed
    }

    pub fn mark_all_read(&self) {
        for entry in self.state().notifications.iter_mut() {
            entry.read = true;
        }
    }

    pub fn clear(&self) {
        self.state().notifications.clear();
    }

    /// Drops everything belonging to one rail entry, so removing a server also removes every trace
    /// of it from the inbox.
    pub fn forget_entry(&self, entry_id: &str) {
        let mut state = self.state();

        state
            .notifications
            .retain(|entry| entry.entry_id != entry_id);
        state.dms.retain(|dm| dm.entry_id != entry_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many of one entry's notifications are still unread, as the feed sees it.
    ///
    /// The feed keeps this — it is what the bell's list is drawn from — but it is no longer what
    /// the rail badge counts: that asks the server what is unread rather than counting what Shiver
    /// happened to collect. So this lives here, where the feed's own behaviour is what is under
    /// test, rather than as a method nothing in the app calls.
    fn unread_for(feed: &Feed, entry_id: &str) -> Option<usize> {
        let count = feed
            .notifications()
            .iter()
            .filter(|entry| entry.entry_id == entry_id && !entry.read)
            .count();

        (count > 0).then_some(count)
    }

    fn dm(
        feed: &Feed,
        entry_id: &str,
        channel_id: Option<i64>,
        author: &str,
        body: &str,
    ) -> bool {
        feed.push(
            entry_id,
            "server",
            RawNotification {
                channel_id,
                channel_name: None,
                author: author.into(),
                body: body.into(),
                icon_url: None,
                is_dm: true,
            },
            false,
        )
    }

    /// A notification, with a body of its own so it is a distinct message rather than a repeat.
    fn note(feed: &Feed, entry_id: &str) {
        note_in(feed, entry_id, None)
    }

    /// One message announced twice must be one line, not two.
    ///
    /// The socket announces what arrives on a server Shiver is watching; opening that server builds
    /// a page, and the page announces the same still-unread message again. The badge went from 1 to
    /// 2 by being clicked on, which is how it was reported.
    #[test]
    fn the_same_message_announced_twice_is_one_entry() {
        let feed = Feed::default();

        assert!(dm(&feed, "a", Some(7), "Smiddy", "hello"), "the first is news");
        assert!(
            !dm(&feed, "a", Some(7), "Smiddy", "hello"),
            "the second is the same message by another route"
        );

        assert_eq!(feed.unread_count(), 1);
    }

    /// And the survivor must be the one that can be cleared.
    ///
    /// The page resolves a direct message's channel by looking the author up in a store that is
    /// empty for the first moments after it connects, so its copy can arrive with no channel at
    /// all — and `mark_channel_read` will not touch one of those. If that copy is the one kept, the
    /// badge can never be got rid of.
    #[test]
    fn a_duplicate_with_no_channel_does_not_cost_the_one_with_a_channel() {
        let feed = Feed::default();

        // the page got there first, without a channel
        assert!(dm(&feed, "a", None, "Smiddy", "hello"));
        // then the socket, which knows it
        assert!(!dm(&feed, "a", Some(7), "Smiddy", "hello"));

        assert!(
            feed.mark_channel_read("a", 7),
            "reading that conversation must clear it"
        );
        assert_eq!(feed.unread_count(), 0);
    }

    #[test]
    fn a_different_message_from_the_same_person_is_still_news() {
        let feed = Feed::default();

        assert!(dm(&feed, "a", Some(7), "Smiddy", "hello"));
        assert!(dm(&feed, "a", Some(7), "Smiddy", "and another thing"));

        assert_eq!(feed.unread_count(), 2);
    }

    #[test]
    fn the_same_words_on_a_different_server_are_not_a_duplicate() {
        let feed = Feed::default();

        assert!(dm(&feed, "a", Some(7), "Smiddy", "hello"));
        assert!(dm(&feed, "b", Some(7), "Smiddy", "hello"));

        assert_eq!(feed.unread_count(), 2);
    }

    /// The rail badge's running half. Losing this is what made every badge vanish: the rail was
    /// switched to the server's own read states alone, which depend on delta events arriving and on
    /// the floor being right, where this depends only on a message turning up.
    #[test]
    fn the_feed_still_counts_per_server_for_the_rail() {
        let feed = Feed::default();

        note(&feed, "a");
        note(&feed, "b");
        note(&feed, "b");

        let counts = feed.unread_by_entry();

        assert_eq!(counts.get("a"), Some(&1));
        assert_eq!(counts.get("b"), Some(&2));
        assert_eq!(counts.get("never-seen"), None, "absent rather than zero");
    }

    #[test]
    fn unread_is_counted_per_server() {
        let feed = Feed::default();

        note(&feed, "a");
        note(&feed, "a");
        note(&feed, "b");

        assert_eq!(unread_for(&feed, "a"), Some(2));
        assert_eq!(unread_for(&feed, "b"), Some(1));
        // a server with nothing unread reads as absent rather than zero
        assert_eq!(unread_for(&feed, "c"), None);
    }

    #[test]
    fn a_muted_channel_never_reaches_a_badge() {
        let feed = Feed::default();

        feed.push(
            "a",
            "server",
            RawNotification {
                channel_id: Some(1),
                channel_name: None,
                author: "someone".into(),
                body: "hi".into(),
                icon_url: None,
                is_dm: false,
            },
            true,
        );

        assert!(feed.notifications().is_empty());
    }

    /// Opening a server is reading it, and must not read anything else.
    #[test]
    fn opening_one_server_clears_only_its_own_badge() {
        let feed = Feed::default();

        note(&feed, "a");
        note(&feed, "b");

        assert!(feed.mark_entry_read("a"));

        assert_eq!(unread_for(&feed, "a"), None);
        assert_eq!(unread_for(&feed, "b"), Some(1));
        assert_eq!(feed.unread_count(), 1);
    }

    #[test]
    fn reading_a_server_with_nothing_unread_changes_nothing() {
        let feed = Feed::default();

        note(&feed, "a");
        feed.mark_entry_read("a");

        // the second read reports no change, so the rail is not told to redraw for nothing
        assert!(!feed.mark_entry_read("a"));
        assert!(!feed.mark_entry_read("never-seen"));
    }

    /// Each call is a *different* message, because that is what these tests mean. The feed now
    /// collapses the same message announced twice, on purpose, so a shared body would quietly turn
    /// "two notifications arrived" into "one did".
    fn note_in(feed: &Feed, entry_id: &str, channel_id: Option<i64>) {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static NEXT: AtomicUsize = AtomicUsize::new(0);

        feed.push(
            entry_id,
            "server",
            RawNotification {
                channel_id,
                channel_name: None,
                author: "someone".into(),
                body: format!("message {}", NEXT.fetch_add(1, Ordering::Relaxed)),
                icon_url: None,
                is_dm: false,
            },
            false,
        );
    }

    /// Reading one channel leaves the rest of the server's unread on the badge.
    #[test]
    fn reading_a_channel_clears_only_that_channel() {
        let feed = Feed::default();

        note_in(&feed, "a", Some(1));
        note_in(&feed, "a", Some(2));
        note_in(&feed, "a", Some(2));

        assert!(feed.mark_channel_read("a", 2));
        assert_eq!(unread_for(&feed, "a"), Some(1));
    }

    #[test]
    fn reading_a_channel_does_not_touch_another_server() {
        let feed = Feed::default();

        note_in(&feed, "a", Some(1));
        note_in(&feed, "b", Some(1));

        feed.mark_channel_read("a", 1);

        assert_eq!(unread_for(&feed, "a"), None);
        assert_eq!(unread_for(&feed, "b"), Some(1));
    }

    /// A notification whose channel could not be resolved could be any channel, so reading one
    /// must not clear it on a guess.
    #[test]
    fn reading_a_channel_leaves_notifications_with_no_channel() {
        let feed = Feed::default();

        note_in(&feed, "a", None);
        note_in(&feed, "a", Some(1));

        assert!(feed.mark_channel_read("a", 1));
        assert_eq!(unread_for(&feed, "a"), Some(1));
    }

    #[test]
    fn reading_an_already_read_channel_reports_no_change() {
        let feed = Feed::default();

        note_in(&feed, "a", Some(1));
        feed.mark_channel_read("a", 1);

        assert!(!feed.mark_channel_read("a", 1));
        assert!(!feed.mark_channel_read("a", 99));
    }

    #[test]
    fn removing_a_server_takes_its_badge_with_it() {
        let feed = Feed::default();

        note(&feed, "a");
        note(&feed, "b");
        feed.forget_entry("a");

        assert_eq!(unread_for(&feed, "a"), None);
        assert_eq!(unread_for(&feed, "b"), Some(1));
    }

    /// The shape that actually came back from a webview, exponent and all.
    ///
    /// A webview returns its numbers as doubles, so a millisecond timestamp arrives as
    /// `1.7698e+12`, which `u64` refuses — and one unreadable field failed the *whole* payload,
    /// taking notifications and mutes with it.
    #[test]
    fn a_timestamp_in_exponential_form_still_parses() {
        let raw = r#"{
            "notifications": [],
            "mutes": [],
            "dms": [{ "channelId": 3, "name": "friend", "lastMessageAt": 1.769871194e+12 }],
            "syncedMutes": null,
            "openDmFailed": null,
            "ready": true,
            "voice": null
        }"#;

        let result: DrainResult = serde_json::from_str(raw).expect("the drain to parse");
        let dms = result.dms.expect("dms");

        assert_eq!(dms[0].last_message_at, Some(1_769_871_194_000));
    }

    /// The whole tick used to be lost to this, not just the timestamp.
    #[test]
    fn a_bad_timestamp_does_not_cost_the_rest_of_the_drain() {
        let raw = r#"{
            "notifications": [{ "author": "someone", "body": "hi", "isDm": false }],
            "mutes": [],
            "ready": true,
            "dms": [{ "channelId": 3, "name": "friend", "lastMessageAt": 1.7698e+12 }]
        }"#;

        let result: DrainResult = serde_json::from_str(raw).expect("the drain to parse");

        assert_eq!(result.notifications.len(), 1);
        assert_eq!(
            result.dms.expect("dms")[0].last_message_at,
            Some(1_769_800_000_000)
        );
    }

    #[test]
    fn a_plain_integer_timestamp_is_unchanged() {
        let raw = r#"{ "channelId": 3, "name": "friend", "lastMessageAt": 1769871194000 }"#;
        let dm: DmChannel = serde_json::from_str(raw).expect("a dm");

        assert_eq!(dm.last_message_at, Some(1_769_871_194_000));
    }

    #[test]
    fn a_missing_or_null_timestamp_is_none() {
        let missing: DmChannel =
            serde_json::from_str(r#"{ "channelId": 3, "name": "friend" }"#).expect("a dm");
        let null: DmChannel =
            serde_json::from_str(r#"{ "channelId": 3, "name": "friend", "lastMessageAt": null }"#)
                .expect("a dm");

        assert_eq!(missing.last_message_at, None);
        assert_eq!(null.last_message_at, None);
    }
}
