//! Shiver's read-only connection to a Sharkord server, spoken from the core rather than a page.
//!
//! Sequence (worked out against a live server; `examples/sharkord.rs probe` reproduces it):
//! 1. `POST /login` -> `{ token }` (done by the caller).
//! 2. Connect to `wss://host/?connectionParams=1`; without the query the server fails the upgrade.
//! 3. Send `{ method: "connectionParams", data: { token } }` to authenticate.
//! 4. Query `others.handshake`, then `others.joinServer` with its hash. Only after the join are
//!    protected procedures allowed; the join returns the initial state, including `readStates`.
//! 5. Query `dms.get` and (if the companion plugin is installed) `plugins.getUserData`.
//! 6. Subscribe to read-state deltas, read-state updates and new messages.
//!
//! It never sends anything on the user's behalf. Unread is accumulated the way Sharkord's own client
//! does: `readStates` from the join is the baseline, deltas add to it, updates set it.

use std::collections::HashMap;

use shiver_core::probe::public_file_url;

pub use crate::check::{check_server, CheckedSessions, ServerCheck};
pub use crate::connection::{retry_delay, watch, Target, Watcher};
pub use crate::error::{Error, Result};
pub use crate::unread::{apply_delta, choose_floor, set_unread, settle_floor, unread_total};

mod check;
mod connection;
mod error;
mod html;
mod unread;
mod wire;

/// One direct-message conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMessage {
    pub channel_id: i64,
    /// resolved here, because the page that draws it belongs to a different server
    pub user_name: String,
    /// the server's own `max(messages.createdAt)` for the conversation, in milliseconds
    pub last_message_at: Option<u64>,
    /// the other person's picture: a signed link, good until the server's token expires
    pub avatar_url: Option<String>,
}

/// A user's picture as the join names it: a file under `/public/`, and the signed link's token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avatar {
    pub name: String,
    pub access_token: Option<String>,
    pub expires_at: Option<u64>,
}

impl Avatar {
    /// Its address on `origin`.
    pub fn url(&self, origin: &str) -> Option<String> {
        let token = self
            .access_token
            .as_deref()
            .map(|token| (token, self.expires_at));

        public_file_url(origin, &self.name, token)
    }
}

/// What the server says about itself and this user when Shiver joins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Joined {
    pub own_user_id: Option<i64>,
    /// unread per channel id, the baseline later events are applied to
    pub read_states: HashMap<i64, u32>,
    pub dm_channels: Vec<i64>,
    pub channel_names: HashMap<i64, String>,
    /// every user's name, since any of them may post next
    pub user_names: HashMap<i64, String>,
    /// the users who have a picture
    pub user_avatars: HashMap<i64, Avatar>,
    pub dms: Vec<DirectMessage>,
    /// the unread floor this user stored through the companion plugin, shared across devices
    pub shared_floor: Option<HashMap<i64, u32>>,
    /// the companion plugin's version, from `pluginsMetadata` (the only place a member can see it)
    pub plugin_version: Option<String>,
}

/// A message that arrived on a watched server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub channel_id: i64,
    /// absent for a message a plugin posted
    pub user_id: Option<i64>,
    pub plugin_id: Option<String>,
    /// the message as plain text
    pub text: String,
    /// the users it mentions
    pub mentioned: Vec<i64>,
}

impl NewMessage {
    /// Whether the user wrote it themselves.
    pub fn is_own(&self, joined: &Joined) -> bool {
        self.user_id.is_some() && self.user_id == joined.own_user_id
    }

    /// Whether it mentions the user.
    pub fn mentions_me(&self, joined: &Joined) -> bool {
        joined
            .own_user_id
            .is_some_and(|own| self.mentioned.contains(&own))
    }

    /// Who wrote it: a plugin by its id, a user by the name the join gave them.
    pub fn author(&self, joined: &Joined) -> String {
        self.plugin_id
            .clone()
            .or_else(|| {
                self.user_id
                    .and_then(|id| joined.user_names.get(&id).cloned())
            })
            .unwrap_or_else(|| "Someone".into())
    }

    /// The text, or what to say for a message that is only an attachment.
    pub fn body(&self) -> &str {
        match self.text.as_str() {
            "" => "Sent an attachment",
            text => text,
        }
    }
}

/// A byte count as people read it: KB below a megabyte, MB with one decimal above.
pub fn readable_size(bytes: usize) -> String {
    match bytes as f64 / (1024.0 * 1024.0) {
        mb if mb < 1.0 => format!("{} KB", bytes / 1024),
        mb => format!("{mb:.1} MB"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// a channel's unread count moved by `delta` (a new message)
    Unread {
        channel_id: i64,
        delta: i64,
    },
    /// a channel's unread count is now `count` (it was read somewhere)
    UnreadSet {
        channel_id: i64,
        count: u32,
    },
    Posted(NewMessage),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_kilobytes_or_megabytes() {
        assert_eq!(readable_size(512 * 1024), "512 KB");
        assert_eq!(readable_size(3 * 1024 * 1024 / 2), "1.5 MB");
    }
}
