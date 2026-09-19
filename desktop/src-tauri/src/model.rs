use serde::{Deserialize, Serialize};

/// One entry in the server rail. The same `origin` may appear more than once so a user can be
/// signed in to one server under several accounts; `id` is what identifies the session everywhere
/// else in Shiver, never the origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub id: String,
    pub origin: String,
    /// `serverId` as reported by GET /info. Two entries sharing it are the same server.
    #[serde(default)]
    pub server_id: Option<String>,
    pub name: String,
    /// absolute url of the server logo, resolved when the entry was added or last refreshed
    #[serde(default)]
    pub icon_url: Option<String>,
    /// the username Shiver signs in with. absent when the user chose to sign in on the server's page.
    #[serde(default)]
    pub identity: Option<String>,
    /// shown under the icon when one origin is added more than once, so the accounts are tellable apart
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub folder_id: Option<String>,
    pub position: i32,
    /// Whether this server may send Shiver larger messages than the default limit allows.
    ///
    /// Off, and it should stay off for a server the user does not run. The limit is there because a
    /// socket buffers whatever arrives, so an oversized frame is memory a server gets to spend on
    /// this machine. Turning this on **raises** the ceiling rather than removing it.
    ///
    /// Only worth touching for a server that has actually tripped the limit, which Shiver reports
    /// in the feed — it is otherwise a silent failure, and silent is how this went unnoticed for a
    /// release last time the number was wrong.
    #[serde(default)]
    pub accept_any_size: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub position: i32,
    #[serde(default)]
    pub expanded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// user-owned theme colours, applied to Shiver's own chrome and handed to the bridge
    pub theme_color: String,
    pub accent_color: String,
    /// The colour text is drawn in, or `None` to take it from the background.
    ///
    /// Absent by default, and absent is not the same as a colour that happens to match: on a light
    /// background Shiver picks dark text and on a dark one light text, and a stored colour would keep
    /// whatever was chosen even after the background moved out from under it. Someone who wants
    /// their own colour says so; everyone else keeps text that follows what they can read.
    #[serde(default)]
    pub text_color: Option<String>,
    /// when on, Shiver plays the notification ping itself and silences the page's own, so a muted
    /// channel is genuinely silent. off leaves each server's client to make its own noise.
    #[serde(default = "default_true")]
    pub notification_sounds: bool,
    /// How loud Shiver's own ping and each server's own sounds are, as a percentage.
    ///
    /// Above 100 is the point of it: both Shiver and Sharkord synthesise their sounds through Web
    /// Audio, and an `<audio>` element cannot be turned past 1.0 — but a gain node can, so a quiet
    /// notification can be made to carry over somebody talking. Voice is untouched: it never
    /// reaches the speakers through this path.
    #[serde(default = "default_sound_volume")]
    pub sound_volume: u16,
    /// Shrink the file card under a picture down to its icon.
    ///
    /// Sharkord renders an image attachment twice — as the picture, and again as a card below it
    /// carrying the filename and the size. On by default: the second copy repeats what the first
    /// already shows, and on a channel of screenshots it is most of the page.
    #[serde(default = "default_true")]
    pub minimise_attachments: bool,
    /// the server Shiver reopens on next launch
    #[serde(default)]
    pub last_server_id: Option<String>,
    /// a system-wide shortcut that mutes the microphone, in Tauri's accelerator form
    /// (`"CommandOrControl+Shift+M"`). Absent means no shortcut is registered at all.
    #[serde(default)]
    pub mute_hotkey: Option<String>,
    /// A version the user asked not to be told about again.
    ///
    /// One version, not a list: the question is only ever "is the newest one the one I turned
    /// down", and a release after it is news again. Skipping is not the same as refusing updates —
    /// there is no setting for that, and this quietly becoming one would be a poor way to get it.
    #[serde(default)]
    pub skipped_update: Option<String>,
    /// How many servers keep a live page, rather than being watched over a socket.
    ///
    /// The memory dial. A page is a whole browser running a whole Sharkord client — on the order of
    /// a hundred megabytes — and it buys one thing: that server is instant to switch back to. A
    /// socket costs almost nothing and reports the same messages, conversations and unread, so every
    /// server above this number is still fully in the inbox; it just has to start its client when
    /// you open it.
    ///
    /// Three by default. Someone with memory to spare and a habit of jumping between many servers
    /// can raise it; someone in thirty servers on a small machine can put it to one and pay for
    /// exactly the server they are looking at. Clamped on the way in — see `pages_kept`.
    #[serde(default = "default_pages_kept")]
    pub pages_kept: u8,
}

/// Sharkord's own dark theme, so Shiver out of the box looks like Sharkord out of the box.
/// These are `--background` (oklch(0.145 0 0)) and `--primary` (oklch(0.922 0 0)) from
/// `apps/client/src/index.css`, converted to hex.
pub const DEFAULT_THEME_COLOR: &str = "#0a0a0a";
pub const DEFAULT_ACCENT_COLOR: &str = "#e5e5e5";

fn default_true() -> bool {
    true
}

/// Unchanged from what the two clients already do, so an existing install sounds the same.
fn default_sound_volume() -> u16 {
    100
}

fn default_pages_kept() -> u8 {
    DEFAULT_PAGES_KEPT
}

/// The default, and the two ends of what the setting may be.
///
/// At least one, because the server on screen always has a page and a zero would be a number Shiver
/// cannot honour. The ceiling is there because this is a memory setting and the whole point of it
/// is a bound — someone typing 500 into a box should get a large number, not an unbounded one.
pub const DEFAULT_PAGES_KEPT: u8 = 3;
pub const MIN_PAGES_KEPT: u8 = 1;
pub const MAX_PAGES_KEPT: u8 = 20;

/// Silent, through to loud enough to hear over a call. Clamped rather than trusted: this is a
/// number a page could put in the settings file, and a gain of 400 is a way to hurt somebody.
pub const MAX_SOUND_VOLUME: u16 = 250;

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_color: DEFAULT_THEME_COLOR.into(),
            accent_color: DEFAULT_ACCENT_COLOR.into(),
            text_color: None,
            notification_sounds: true,
            sound_volume: 100,
            minimise_attachments: true,
            last_server_id: None,
            mute_hotkey: None,
            pages_kept: DEFAULT_PAGES_KEPT,
            skipped_update: None,
        }
    }
}

/// A colour Shiver is willing to put in a stylesheet, or the default in its place.
///
/// **This is the check that was missing.** These are plain strings, they are accepted wholesale by
/// `update_settings` and they live in a file a person can edit — and the bridge interpolates them
/// straight into a `<style>` element's `textContent`, which is parsed as CSS. A value containing
/// `}` closes the rule block, and everything after it is arbitrary CSS injected into every Sharkord
/// page. The neighbouring `sound_volume` is clamped with a comment making exactly this argument
/// about exactly this file; the colours never got it.
///
/// Six hex digits with a leading `#`, which is all the colour input can produce and all either
/// side knows how to read.
pub fn sanitised_color(value: &str, fallback: &str) -> String {
    if is_hex_color(value) {
        return value.to_ascii_lowercase();
    }

    eprintln!("[shiver] '{value}' is not a colour Shiver will use, falling back to {fallback}");

    fallback.to_string()
}

/// The same rule, for the optional text colour: kept when it is a colour, dropped when it is not.
pub fn sanitised_optional_color(value: Option<&str>) -> Option<String> {
    match value {
        Some(value) if is_hex_color(value) => Some(value.to_ascii_lowercase()),
        Some(value) => {
            eprintln!("[shiver] '{value}' is not a colour Shiver will use, so text follows the background");

            None
        }
        None => None,
    }
}

fn is_hex_color(value: &str) -> bool {
    let Some(digits) = value.strip_prefix('#') else {
        return false;
    };

    digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl Settings {
    /// Every colour in this struct, held to what is safe to put in a stylesheet.
    ///
    /// Applied on the way in from the settings screen and on the way out of the file, so a value
    /// that was already stored by an older build cannot reach the bridge either.
    pub fn sanitised(mut self) -> Self {
        self.theme_color = sanitised_color(&self.theme_color, DEFAULT_THEME_COLOR);
        self.accent_color = sanitised_color(&self.accent_color, DEFAULT_ACCENT_COLOR);
        self.text_color = sanitised_optional_color(self.text_color.as_deref());
        self.sound_volume = self.sound_volume.min(MAX_SOUND_VOLUME);
        self.pages_kept = self.pages_kept.clamp(MIN_PAGES_KEPT, MAX_PAGES_KEPT);

        self
    }

    /// The setting, held to what Shiver can actually honour.
    ///
    /// Clamped at the point of use rather than trusted from the file: `settings.json` is a file on
    /// disk a person can edit, and a zero there would mean closing the page the user is looking at.
    pub fn pages_kept(&self) -> usize {
        self.pages_kept.clamp(MIN_PAGES_KEPT, MAX_PAGES_KEPT) as usize
    }

    /// True while the user has not picked their own colours. Shiver injects no css into a server's
    /// page in that case, so a stock server looks exactly as its own client intends.
    pub fn uses_default_colors(&self) -> bool {
        self.theme_color.eq_ignore_ascii_case(DEFAULT_THEME_COLOR)
            && self.accent_color.eq_ignore_ascii_case(DEFAULT_ACCENT_COLOR)
            && self.text_color.is_none()
    }
}

/// A channel the user has muted, scoped to one rail entry so muting a channel on one account never
/// silences the same channel viewed from another.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutedChannel {
    pub entry_id: String,
    pub channel_id: i64,
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
    /// entry id -> that server's unread floor, per channel id, as of the last time the user read it.
    ///
    /// **What makes a badge survive a restart.** A badge used to count only the notifications Shiver
    /// had collected in this run, so closing it threw the count away and reopening it started from
    /// nothing — whatever arrived in between was never mentioned. Sharkord keeps per-channel unread
    /// of its own and hands it over on connect; measured against this floor, the difference is
    /// everything that has arrived since the user last opened that server, including while Shiver
    /// was closed.
    ///
    /// A floor rather than a plain "count what the server says", because what the server says
    /// includes every message in every channel the user has never opened — on a public server, its
    /// whole history. The floor is taken the first time Shiver connects and re-taken when the user
    /// opens that server, which is the one moment "you have seen this" is actually true.
    ///
    /// In the registry rather than the keychain: it is a count of unread messages, not a secret.
    #[serde(default)]
    pub baselines: std::collections::HashMap<String, std::collections::HashMap<i64, u32>>,
}

/// What GET /info exposes to a client that has not signed in yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub origin: String,
    pub server_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
}

/// The origin rules, which live in `shared/shiver-core` so there is one copy rather than two.
///
/// These were byte-identical in both clients, tests included, and `is_same_origin` is the whole of
/// Shiver's webview pinning — the single comparison that decides whether a page may navigate
/// somewhere. Re-exported rather than moved out of sight, so every existing caller reads the same.
pub use shiver_core::{is_same_origin, normalize_origin};

#[cfg(test)]
mod tests {}
