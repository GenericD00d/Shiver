use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, Result};

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
        }
    }
}

impl Settings {
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

/// Reduces a user-typed address to a bare `scheme://host[:port]` origin.
///
/// Every Shiver security boundary is keyed on this string, so it rejects anything that could make
/// two different servers compare equal: embedded credentials and paths. It also rejects any scheme
/// but https, which is a separate rule with its own reason — see below.
pub fn normalize_origin(input: &str) -> Result<String> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(Error::InvalidOrigin("Enter a server address".into()));
    }

    // a bare host is the common case when typing an address, and https is the only scheme Shiver
    // accepts, so there is nothing to guess
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    let url = Url::parse(&candidate)
        .map_err(|_| Error::InvalidOrigin(format!("'{trimmed}' is not a valid address")))?;

    // https only, and no exception for localhost or a private address. The reason is not only the
    // wire: Shiver signing a user in over http would make cleartext credentials a supported way to
    // use it, and that is not a thing this client should teach anyone to accept.
    if url.scheme() != "https" {
        return Err(Error::InvalidOrigin(
            "Shiver only connects over https. An http:// address would put your password and session on the wire in the clear.".into(),
        ));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::InvalidOrigin(
            "Addresses must not contain a username or password".into(),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| Error::InvalidOrigin(format!("'{trimmed}' has no host")))?;

    match url.port() {
        Some(port) => Ok(format!("{}://{}:{}", url.scheme(), host, port)),
        None => Ok(format!("{}://{}", url.scheme(), host)),
    }
}

/// True when `url` belongs to `origin`. This is the whole of Shiver's origin pinning: a webview is
/// allowed to navigate within the server it was opened for and nowhere else.
pub fn is_same_origin(origin: &str, url: &Url) -> bool {
    let host = match url.host_str() {
        Some(host) => host,
        None => return false,
    };

    let candidate = match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    };

    candidate == origin
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_a_bare_host_to_https() {
        assert_eq!(
            normalize_origin("chat.example.com").unwrap(),
            "https://chat.example.com"
        );
    }

    /// The two ways a person actually types an address. Both must land on the same origin, since
    /// every security boundary in Shiver is keyed on that string.
    #[test]
    fn both_ways_of_typing_an_address_agree() {
        let expected = "https://chat.example.com";

        for typed in [
            "chat.example.com",
            "https://chat.example.com",
            "https://chat.example.com/",
            "  chat.example.com  ",
            "HTTPS://chat.example.com",
            "https://chat.example.com/channels/12",
        ] {
            assert_eq!(
                normalize_origin(typed).unwrap(),
                expected,
                "'{typed}' should normalise to {expected}"
            );
        }
    }

    #[test]
    fn keeps_an_explicit_port_and_drops_the_path() {
        assert_eq!(
            normalize_origin("https://localhost:4991/channels/1").unwrap(),
            "https://localhost:4991"
        );
    }

    /// The rule the user asked for, in every shape it can be typed. There is deliberately no
    /// exemption for localhost or a private address: an http address is refused everywhere.
    #[test]
    fn refuses_plain_http_however_it_is_written() {
        for address in [
            "http://chat.example.com",
            "HTTP://chat.example.com",
            "http://localhost:4991",
            "http://127.0.0.1:4991",
            "http://192.168.1.10",
        ] {
            let refused = normalize_origin(address);

            assert!(refused.is_err(), "{address} should have been refused");
            assert!(
                refused.unwrap_err().to_string().contains("https"),
                "{address}: the refusal should say what is wanted instead"
            );
        }
    }

    #[test]
    fn rejects_credentials_and_foreign_schemes() {
        assert!(normalize_origin("https://user:pw@example.com").is_err());
        assert!(normalize_origin("ftp://example.com").is_err());
        assert!(normalize_origin("   ").is_err());
    }

    #[test]
    fn same_origin_is_exact_on_scheme_host_and_port() {
        let origin = "https://example.com";

        assert!(is_same_origin(
            origin,
            &Url::parse("https://example.com/a/b").unwrap()
        ));
        assert!(!is_same_origin(
            origin,
            &Url::parse("http://example.com").unwrap()
        ));
        assert!(!is_same_origin(
            origin,
            &Url::parse("https://example.com:8443").unwrap()
        ));
        assert!(!is_same_origin(
            origin,
            &Url::parse("https://evil.example.com").unwrap()
        ));
    }
}
