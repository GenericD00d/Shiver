//! The unified notification and DM feed, filled from page drains and from core sockets.
//!
//! Everything in it came from a server and is bounded on the way in: field lengths, entries per
//! server, and entries overall.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use shiver_core::{text::clamp, LockExt};

use crate::voice::VoiceSnapshot;

/// Entries kept overall, and per server, so one noisy server cannot evict everyone else's.
const MAX_ENTRIES: usize = 300;
const MAX_PER_SERVER: usize = 100;

const MAX_AUTHOR: usize = 100;
const MAX_BODY: usize = 500;
const MAX_CHANNEL_NAME: usize = 100;
const MAX_URL: usize = 2048;
const MAX_DMS: usize = 500;

/// Two identical notifications from one server this close together are one message arriving by two
/// routes (socket and page). Short, so someone repeating themselves later still gets two lines.
const DUPLICATE_WINDOW_MS: u64 = 30 * 1000;

/// Milliseconds from a page, where numbers are doubles (possibly in exponent form).
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
            Millis::Float(millis) if millis.is_finite() && millis >= 0.0 => {
                Some(millis.round() as u64)
            }
            Millis::Float(_) => None,
        }),
    )
}

/// An icon url from a page, kept only if it is https on the server that sent it.
fn safe_icon_url(url: &str, origin: Option<&str>) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;

    crate::model::is_same_origin(origin?, &parsed).then(|| clamp(url.to_string(), MAX_URL))
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
    /// set on Shiver's own "new version" entry, which the popup draws with a button
    #[serde(default)]
    pub update: Option<String>,
}

/// A DM conversation as reported by one server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmChannel {
    pub channel_id: i64,
    pub name: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default, deserialize_with = "optional_millis")]
    pub last_message_at: Option<u64>,
}

/// A conversation plus the rail entry (account) it belongs to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmEntry {
    pub entry_id: String,
    pub server_name: String,
    pub account_label: String,
    pub channel: DmChannel,
}

/// What a page hands back when drained. Every field is the page's claim, not a fact.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainResult {
    #[serde(default)]
    pub notifications: Vec<RawNotification>,
    /// absent when nothing changed
    #[serde(default)]
    pub dms: Option<Vec<DmChannel>>,
    /// mutes toggled from Sharkord's channel menu
    #[serde(default)]
    pub mutes: Vec<QueuedMute>,
    /// the plugin's reconciled mute list, once per connect
    #[serde(default)]
    pub synced_mutes: Option<Vec<i64>>,
    #[serde(default)]
    pub open_dm_failed: Option<String>,
    /// links the page wants opened in the browser
    #[serde(default)]
    pub open: Vec<String>,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub signed_out: bool,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub voice: Option<VoiceSnapshot>,
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
    /// newest first
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

impl FeedState {
    fn insert(&mut self, notification: Notification) {
        let entry_id = notification.entry_id.clone();

        self.notifications.push_front(notification);

        // drop the oldest of this server's entries beyond its share, then the oldest overall
        let mut seen = 0;

        self.notifications.retain(|entry| {
            if entry.entry_id != entry_id {
                return true;
            }

            seen += 1;

            seen <= MAX_PER_SERVER
        });
        self.notifications.truncate(MAX_ENTRIES);
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;

        self.next_id
    }
}

impl Feed {
    /// Adds one notification. Returns false when it was muted or a duplicate.
    ///
    /// A duplicate (same server, author and body within `DUPLICATE_WINDOW_MS`) is merged, keeping
    /// whichever copy knows its channel, since only that one can be cleared by reading the channel.
    pub fn push(
        &self,
        entry_id: &str,
        server_name: &str,
        origin: Option<&str>,
        raw: RawNotification,
        muted: bool,
    ) -> bool {
        if muted {
            return false;
        }

        let author = clamp(raw.author, MAX_AUTHOR);
        let body = clamp(raw.body, MAX_BODY);
        let channel_name = raw.channel_name.map(|name| clamp(name, MAX_CHANNEL_NAME));
        let now = now_ms();
        let mut state = self.0.locked();

        if let Some(existing) = state.notifications.iter_mut().find(|entry| {
            !entry.read
                && entry.entry_id == entry_id
                && entry.author == author
                && entry.body == body
                && now.saturating_sub(entry.at) < DUPLICATE_WINDOW_MS
        }) {
            if existing.channel_id.is_none() {
                existing.channel_id = raw.channel_id;
                existing.channel_name = channel_name;
            }

            return false;
        }

        let id = state.next_id();

        state.insert(Notification {
            id,
            entry_id: entry_id.to_string(),
            server_name: server_name.to_string(),
            channel_id: raw.channel_id,
            channel_name,
            author,
            body,
            icon_url: raw.icon_url.and_then(|url| safe_icon_url(&url, origin)),
            is_dm: raw.is_dm,
            at: now,
            read: false,
            update: None,
        });

        true
    }

    pub fn forget_updates(&self) {
        self.0
            .locked()
            .notifications
            .retain(|entry| entry.update.is_none());
    }

    /// Offers a new version, replacing any earlier offer.
    pub fn push_update(&self, version: &str) {
        let mut state = self.0.locked();

        state.notifications.retain(|entry| entry.update.is_none());

        let id = state.next_id();

        state.insert(Notification {
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
    }

    /// Replaces one entry's whole DM list (bounded).
    pub fn set_dms(
        &self,
        entry_id: &str,
        server_name: &str,
        account_label: &str,
        origin: Option<&str>,
        channels: Vec<DmChannel>,
    ) {
        let mut state = self.0.locked();

        state.dms.retain(|dm| dm.entry_id != entry_id);
        state
            .dms
            .extend(channels.into_iter().take(MAX_DMS).map(|channel| DmEntry {
                entry_id: entry_id.to_string(),
                server_name: server_name.to_string(),
                account_label: account_label.to_string(),
                channel: DmChannel {
                    name: clamp(channel.name, MAX_AUTHOR),
                    icon_url: channel.icon_url.and_then(|url| safe_icon_url(&url, origin)),
                    ..channel
                },
            }));
    }

    pub fn notifications(&self) -> Vec<Notification> {
        self.0.locked().notifications.iter().cloned().collect()
    }

    pub fn dms(&self) -> Vec<DmEntry> {
        self.0.locked().dms.clone()
    }

    pub fn unread_count(&self) -> usize {
        self.0
            .locked()
            .notifications
            .iter()
            .filter(|entry| !entry.read)
            .count()
    }

    /// Unread per entry; entries with none are absent.
    pub fn unread_by_entry(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();

        for entry in self
            .0
            .locked()
            .notifications
            .iter()
            .filter(|entry| !entry.read)
        {
            *counts.entry(entry.entry_id.clone()).or_default() += 1;
        }

        counts
    }

    /// Marks matching unread entries read; returns whether any changed.
    fn mark_read(&self, matches: impl Fn(&Notification) -> bool) -> bool {
        let mut changed = false;

        for entry in self
            .0
            .locked()
            .notifications
            .iter_mut()
            .filter(|entry| !entry.read && matches(entry))
        {
            entry.read = true;
            changed = true;
        }

        changed
    }

    /// One channel's notifications (those with no known channel are left alone).
    pub fn mark_channel_read(&self, entry_id: &str, channel_id: i64) -> bool {
        self.mark_read(|entry| entry.entry_id == entry_id && entry.channel_id == Some(channel_id))
    }

    pub fn mark_entry_read(&self, entry_id: &str) -> bool {
        self.mark_read(|entry| entry.entry_id == entry_id)
    }

    pub fn mark_all_read(&self) {
        self.mark_read(|_| true);
    }

    pub fn clear(&self) {
        self.0.locked().notifications.clear();
    }

    /// Drops everything belonging to one entry.
    pub fn forget_entry(&self, entry_id: &str) {
        let mut state = self.0.locked();

        state
            .notifications
            .retain(|entry| entry.entry_id != entry_id);
        state.dms.retain(|dm| dm.entry_id != entry_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(channel_id: Option<i64>, author: &str, body: &str) -> RawNotification {
        RawNotification {
            channel_id,
            channel_name: None,
            author: author.into(),
            body: body.into(),
            icon_url: None,
            is_dm: false,
        }
    }

    fn unread_for(feed: &Feed, entry_id: &str) -> usize {
        feed.unread_by_entry().get(entry_id).copied().unwrap_or(0)
    }

    #[test]
    fn a_message_announced_twice_is_one_entry_and_keeps_its_channel() {
        let feed = Feed::default();

        assert!(feed.push("a", "s", None, raw(None, "Smiddy", "hello"), false));
        assert!(!feed.push("a", "s", None, raw(Some(7), "Smiddy", "hello"), false));
        assert!(feed.push("a", "s", None, raw(Some(7), "Smiddy", "another"), false));
        assert!(feed.push("b", "s", None, raw(Some(7), "Smiddy", "hello"), false));

        assert!(feed.mark_channel_read("a", 7));
        assert_eq!(feed.unread_count(), 1);
    }

    #[test]
    fn reading_is_scoped_to_the_channel_or_server() {
        let feed = Feed::default();

        feed.push("a", "s", None, raw(Some(1), "x", "1"), false);
        feed.push("a", "s", None, raw(Some(2), "x", "2"), false);
        feed.push("a", "s", None, raw(None, "x", "3"), false);
        feed.push("b", "s", None, raw(Some(1), "x", "4"), false);

        assert!(feed.mark_channel_read("a", 2));
        assert!(!feed.mark_channel_read("a", 2));
        assert_eq!((unread_for(&feed, "a"), unread_for(&feed, "b")), (2, 1));

        assert!(feed.mark_entry_read("a"));
        assert_eq!((unread_for(&feed, "a"), unread_for(&feed, "b")), (0, 1));

        feed.forget_entry("b");
        assert_eq!(feed.unread_count(), 0);
    }

    #[test]
    fn a_muted_channel_never_reaches_the_feed() {
        let feed = Feed::default();

        assert!(!feed.push("a", "s", None, raw(Some(1), "x", "y"), true));
        assert!(feed.notifications().is_empty());
    }

    #[test]
    fn one_server_cannot_evict_another() {
        let feed = Feed::default();

        feed.push("quiet", "s", None, raw(None, "x", "keep me"), false);

        for index in 0..1000 {
            feed.push(
                "noisy",
                "s",
                None,
                raw(None, "x", &index.to_string()),
                false,
            );
        }

        assert_eq!(unread_for(&feed, "quiet"), 1);
        assert_eq!(unread_for(&feed, "noisy"), MAX_PER_SERVER);
    }

    #[test]
    fn fields_are_bounded_by_characters() {
        let feed = Feed::default();

        feed.push(
            "a",
            "s",
            None,
            raw(None, &"é".repeat(500), &"x".repeat(5000)),
            false,
        );

        let entry = &feed.notifications()[0];

        assert_eq!(entry.author.chars().count(), MAX_AUTHOR + 1);
        assert_eq!(entry.body.chars().count(), MAX_BODY + 1);
    }

    #[test]
    fn icons_must_be_https_on_the_sending_server() {
        assert!(safe_icon_url("https://a.example/x.png", Some("https://a.example")).is_some());
        assert!(safe_icon_url("https://b.example/x.png", Some("https://a.example")).is_none());
        assert!(safe_icon_url("http://a.example/x.png", Some("https://a.example")).is_none());
        assert!(safe_icon_url("https://a.example/x.png", None).is_none());
    }

    #[test]
    fn timestamps_in_exponent_form_still_parse() {
        let result: DrainResult = serde_json::from_str(
            r#"{ "notifications": [{ "author": "a", "body": "b" }],
                 "dms": [{ "channelId": 3, "name": "friend", "lastMessageAt": 1.769871194e+12 }] }"#,
        )
        .unwrap();

        assert_eq!(result.notifications.len(), 1);
        assert_eq!(
            result.dms.unwrap()[0].last_message_at,
            Some(1_769_871_194_000)
        );

        let dm: DmChannel =
            serde_json::from_str(r#"{ "channelId": 3, "name": "f", "lastMessageAt": null }"#)
                .unwrap();

        assert_eq!(dm.last_message_at, None);
    }
}
