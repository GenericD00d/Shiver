//! The registry saved in `servers.json`. Nothing secret lives here; credentials are in the keychain.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub use shiver_core::{
    is_same_origin,
    model::{
        sanitised_color, sanitised_optional_color, Folder, MutedChannel, DEFAULT_ACCENT_COLOR,
        DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME,
    },
    normalize_origin,
    probe::ServerInfo,
    rail::Rail,
};

shiver_core::rail_server!(ServerEntry);

/// One rail entry. The same origin may appear more than once (several accounts on one server);
/// `id` is what identifies the entry everywhere, never the origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub id: String,
    pub origin: String,
    /// `serverId` from `/info`
    #[serde(default)]
    pub server_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    /// the username Shiver signs in with; absent when the user signs in on the server's own page
    #[serde(default)]
    pub identity: Option<String>,
    /// shown under the icon so several accounts on one server can be told apart
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub folder_id: Option<String>,
    pub position: i32,
    /// raises (never removes) the websocket message size limit for a server the user trusts
    #[serde(default)]
    pub accept_any_size: bool,
    /// the generation of this entry's browser profile; replaced on log out so nothing carries over
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme_color: String,
    pub accent_color: String,
    /// `None` picks dark or light text from the background
    #[serde(default)]
    pub text_color: Option<String>,
    /// Shiver plays the ping itself (so muted channels stay silent) instead of each page
    #[serde(default = "default_true")]
    pub notification_sounds: bool,
    /// percent; above 100 works because both Shiver and Sharkord synthesise sounds via Web Audio
    #[serde(default = "default_sound_volume")]
    pub sound_volume: u16,
    /// shrink the file card Sharkord draws under an image to its icon
    #[serde(default = "default_true")]
    pub minimise_attachments: bool,
    #[serde(default)]
    pub last_server_id: Option<String>,
    /// a global mute shortcut in Tauri accelerator form, e.g. `CommandOrControl+Shift+M`
    #[serde(default)]
    pub mute_hotkey: Option<String>,
    /// the one release the user asked not to be told about again
    #[serde(default)]
    pub skipped_update: Option<String>,
    /// how many servers keep a live page; the rest are watched over a socket
    #[serde(default = "default_pages_kept")]
    pub pages_kept: u8,
}

pub const DEFAULT_PAGES_KEPT: u8 = 3;
pub const MIN_PAGES_KEPT: u8 = 1;
pub const MAX_PAGES_KEPT: u8 = 20;

fn default_true() -> bool {
    true
}

fn default_sound_volume() -> u16 {
    100
}

fn default_pages_kept() -> u8 {
    DEFAULT_PAGES_KEPT
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_color: DEFAULT_THEME_COLOR.into(),
            accent_color: DEFAULT_ACCENT_COLOR.into(),
            text_color: None,
            notification_sounds: true,
            sound_volume: default_sound_volume(),
            minimise_attachments: true,
            last_server_id: None,
            mute_hotkey: None,
            pages_kept: DEFAULT_PAGES_KEPT,
            skipped_update: None,
        }
    }
}

impl Settings {
    /// Every field held to a valid, safe value; applied on the way in and out of the file.
    pub fn sanitised(mut self) -> Self {
        self.theme_color = sanitised_color(&self.theme_color, DEFAULT_THEME_COLOR);
        self.accent_color = sanitised_color(&self.accent_color, DEFAULT_ACCENT_COLOR);
        self.text_color = sanitised_optional_color(self.text_color.as_deref());
        self.sound_volume = self.sound_volume.min(MAX_SOUND_VOLUME);
        self.pages_kept = self.pages_kept.clamp(MIN_PAGES_KEPT, MAX_PAGES_KEPT);
        self
    }

    pub fn pages_kept(&self) -> usize {
        self.pages_kept.clamp(MIN_PAGES_KEPT, MAX_PAGES_KEPT) as usize
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub muted: Vec<MutedChannel>,
    /// entry id -> per-channel unread counts when Shiver first saw that server. The badge counts
    /// only what arrives above this floor, so a server's old backlog is not "unread".
    #[serde(default)]
    pub baselines: HashMap<String, HashMap<i64, u32>>,
    /// entries whose camera/microphone answers should be reset the next time their page opens
    #[serde(default)]
    pub pending_permission_resets: Vec<String>,
}

impl Registry {
    pub fn server(&self, id: &str) -> Option<&ServerEntry> {
        self.servers.iter().find(|server| server.id == id)
    }

    pub fn server_mut(&mut self, id: &str) -> Option<&mut ServerEntry> {
        self.servers.iter_mut().find(|server| server.id == id)
    }

    pub fn muted_for(&self, entry_id: &str) -> Vec<i64> {
        shiver_core::model::muted_for(&self.muted, entry_id)
    }

    /// Replaces one entry's mutes; returns the new list.
    pub fn set_muted_for(
        &mut self,
        entry_id: &str,
        channels: impl IntoIterator<Item = i64>,
    ) -> Vec<i64> {
        shiver_core::model::set_muted_for(&mut self.muted, entry_id, channels)
    }

    /// The next free top-level position.
    pub fn next_position(&self) -> i32 {
        shiver_core::rail::next_position(&self.servers, &self.folders)
    }

    pub fn rail(&mut self) -> Rail<'_, ServerEntry> {
        Rail {
            servers: &mut self.servers,
            folders: &mut self.folders,
        }
    }
}
