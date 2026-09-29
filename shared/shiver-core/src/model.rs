//! Registry pieces both clients store identically.

use serde::{Deserialize, Serialize};

/// Sharkord's own dark theme (`--background` and `--primary`), so a stock Shiver restyles nothing.
pub const DEFAULT_THEME_COLOR: &str = "#0a0a0a";
pub const DEFAULT_ACCENT_COLOR: &str = "#e5e5e5";
/// Sharkord's sidebar shade, kept for the rail on the default theme (`DEFAULT_RAIL_COLOR` in
/// `shared/web/settings.ts`).
pub const DEFAULT_RAIL_COLOR: &str = "#171717";

/// A sound gain above this could hurt someone wearing headphones.
pub const MAX_SOUND_VOLUME: u16 = 250;

/// serde defaults both clients' settings use.
pub fn default_true() -> bool {
    true
}

pub fn default_sound_volume() -> u16 {
    100
}

/// A folder in the rail.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub position: i32,
    #[serde(default)]
    pub expanded: bool,
}

/// A channel muted on one rail entry only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MutedChannel {
    pub entry_id: String,
    pub channel_id: i64,
}

fn is_hex_color(value: &str) -> bool {
    value.strip_prefix('#').is_some_and(|digits| {
        digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

/// `#rrggbb` (lowercased), or `fallback`. Colours become CSS in every server page, so nothing else
/// may pass.
/// Which of a server's messages notify: reach the bell, a system notification or the phone's.
/// Unread badges count every message either way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotifyLevel {
    #[default]
    All,
    /// messages that mention the user, and DMs
    Mentions,
    Dms,
}

impl NotifyLevel {
    pub fn allows(self, is_dm: bool, mentions_me: bool) -> bool {
        match self {
            Self::All => true,
            Self::Mentions => is_dm || mentions_me,
            Self::Dms => is_dm,
        }
    }
}

/// Anything unreadable (a value from a newer Shiver) reads as `All`, rather than failing the
/// whole registry.
impl<'de> Deserialize<'de> for NotifyLevel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(
            match serde_json::Value::deserialize(deserializer)?.as_str() {
                Some("mentions") => Self::Mentions,
                Some("dms") => Self::Dms,
                _ => Self::All,
            },
        )
    }
}

pub fn sanitised_color(value: &str, fallback: &str) -> String {
    if is_hex_color(value) {
        value.to_ascii_lowercase()
    } else {
        fallback.to_string()
    }
}

pub fn sanitised_optional_color(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| is_hex_color(value))
        .map(str::to_ascii_lowercase)
}

/// Whether colours are Sharkord's own, in which case Shiver injects no css into pages.
fn uses_default_colors(theme: &str, accent: &str, text: Option<&str>) -> bool {
    theme.eq_ignore_ascii_case(DEFAULT_THEME_COLOR)
        && accent.eq_ignore_ascii_case(DEFAULT_ACCENT_COLOR)
        && text.is_none()
}

/// The red, green and blue of a `#rrggbb` colour.
pub fn rgb(value: &str) -> Option<[u8; 3]> {
    if !is_hex_color(value) {
        return None;
    }

    let channel = |at: usize| u8::from_str_radix(&value[at..at + 2], 16).ok();

    Some([channel(1)?, channel(3)?, channel(5)?])
}

/// Whether dark text reads better than light on `color` (relative luminance, as
/// `shared/web/colors.ts` decides it).
fn is_light([r, g, b]: [u8; 3]) -> bool {
    let channel = |value: u8| {
        let srgb = f64::from(value) / 255.0;

        if srgb <= 0.03928 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    };

    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b) > 0.35
}

/// `color` mixed towards black (a light colour) or white (a dark one), keeping `percent` of it:
/// `lift` in `shared/web/colors.ts`.
fn lift(color: [u8; 3], percent: u8) -> [u8; 3] {
    let toward = if is_light(color) { 0.0 } else { 255.0 };
    let share = f64::from(percent.min(100)) / 100.0;

    color.map(|channel| (f64::from(channel) * share + toward * (1.0 - share)).round() as u8)
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Shiver's own colours for something drawn natively rather than in its page, as
/// `shared/web/theme.ts` derives them there: `#rrggbb` each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub rail: String,
    pub surface: String,
    pub surface_dim: String,
    pub text: String,
    pub accent: String,
}

pub fn palette(theme: &str, accent: &str, text: Option<&str>) -> Palette {
    let theme_hex = sanitised_color(theme, DEFAULT_THEME_COLOR);
    let theme = rgb(&theme_hex).unwrap_or([0x0a; 3]);
    let rail = if theme_hex == DEFAULT_THEME_COLOR {
        DEFAULT_RAIL_COLOR.to_string()
    } else {
        theme_hex
    };
    let automatic = if is_light(theme) {
        "#171717"
    } else {
        "#fafafa"
    };

    Palette {
        rail,
        surface: hex(lift(theme, 84)),
        surface_dim: hex(lift(theme, 92)),
        text: sanitised_optional_color(text).unwrap_or_else(|| automatic.to_string()),
        accent: sanitised_color(accent, DEFAULT_ACCENT_COLOR),
    }
}

/// The theme handed to server pages: `null` on Sharkord's own colours (pages are left untouched),
/// otherwise sanitised colours only, since the bridge turns them into CSS.
pub fn theme_payload(theme: &str, accent: &str, text: Option<&str>) -> serde_json::Value {
    if uses_default_colors(theme, accent, text) {
        return serde_json::Value::Null;
    }

    serde_json::json!({
        "themeColor": sanitised_color(theme, DEFAULT_THEME_COLOR),
        "accentColor": sanitised_color(accent, DEFAULT_ACCENT_COLOR),
        "textColor": sanitised_optional_color(text),
    })
}

/// Channel ids muted on one entry.
pub fn muted_for(muted: &[MutedChannel], entry_id: &str) -> Vec<i64> {
    muted
        .iter()
        .filter(|muted| muted.entry_id == entry_id)
        .map(|muted| muted.channel_id)
        .collect()
}

/// A page can report mutes, so their number is bounded.
const MAX_MUTED_PER_ENTRY: usize = 500;

/// Mutes as stored: sorted, deduplicated, bounded.
pub fn normalized_mutes(channels: impl IntoIterator<Item = i64>) -> Vec<i64> {
    let mut kept: Vec<i64> = channels.into_iter().collect();

    kept.sort_unstable();
    kept.dedup();
    kept.truncate(MAX_MUTED_PER_ENTRY);

    kept
}

/// Replaces one entry's mutes (normalised); returns the new list.
pub fn set_muted_for(
    muted: &mut Vec<MutedChannel>,
    entry_id: &str,
    channels: impl IntoIterator<Item = i64>,
) -> Vec<i64> {
    muted.retain(|muted| muted.entry_id != entry_id);

    let kept = normalized_mutes(channels);

    muted.extend(kept.iter().map(|channel_id| MutedChannel {
        entry_id: entry_id.to_string(),
        channel_id: *channel_id,
    }));

    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_six_digit_hex_colours_have_channels() {
        assert_eq!(rgb("#0a0A0f"), Some([10, 10, 15]));

        for bad in ["", "0a0a0a", "#abc", "#zzzzzz", "#0a0a0a0a", "#+1234a"] {
            assert_eq!(rgb(bad), None, "{bad}");
        }
    }

    #[test]
    fn default_colours_restyle_nothing() {
        assert!(theme_payload(DEFAULT_THEME_COLOR, DEFAULT_ACCENT_COLOR, None).is_null());
        assert_eq!(
            theme_payload("#FFFFFF", "bad", None)["accentColor"],
            DEFAULT_ACCENT_COLOR
        );
    }

    #[test]
    fn only_six_digit_hex_colours_survive() {
        assert_eq!(sanitised_color("#ABCDEF", "#000000"), "#abcdef");
        assert_eq!(sanitised_color("#fff", "#000000"), "#000000");
        assert_eq!(
            sanitised_color("#0a0a0a} html { display: none }", "#000000"),
            "#000000"
        );
        assert_eq!(
            sanitised_optional_color(Some("#123456")),
            Some("#123456".into())
        );
        assert_eq!(
            sanitised_optional_color(Some("#ff0000; content: 'x'")),
            None
        );
    }

    #[test]
    fn mutes_are_replaced_per_entry_deduplicated_and_capped() {
        let mut muted = Vec::new();

        set_muted_for(&mut muted, "b", [9]);
        assert_eq!(set_muted_for(&mut muted, "a", [3, 3, 1]), vec![1, 3]);
        assert_eq!(muted_for(&muted, "b"), vec![9]);
        assert_eq!(
            set_muted_for(&mut muted, "a", 0..10_000).len(),
            MAX_MUTED_PER_ENTRY
        );
    }

    #[test]
    fn a_notify_level_reads_tolerantly_and_decides() {
        let read = |json: &str| serde_json::from_str::<NotifyLevel>(json).unwrap();

        assert_eq!(read(r#""mentions""#), NotifyLevel::Mentions);
        assert_eq!(read(r#""dms""#), NotifyLevel::Dms);
        assert_eq!(read(r#""everything""#), NotifyLevel::All);
        assert_eq!(read("7"), NotifyLevel::All);
        assert_eq!(
            serde_json::to_string(&NotifyLevel::Dms).unwrap(),
            r#""dms""#
        );

        assert!(NotifyLevel::All.allows(false, false));
        assert!(NotifyLevel::Mentions.allows(false, true));
        assert!(NotifyLevel::Mentions.allows(true, false));
        assert!(!NotifyLevel::Mentions.allows(false, false));
        assert!(!NotifyLevel::Dms.allows(false, true));
        assert!(NotifyLevel::Dms.allows(true, false));
    }

    #[test]
    fn a_native_palette_follows_the_page_theme() {
        let stock = palette(DEFAULT_THEME_COLOR, DEFAULT_ACCENT_COLOR, None);

        assert_eq!(stock.rail, DEFAULT_RAIL_COLOR);
        assert_eq!(stock.text, "#fafafa");
        // 84% of #0a0a0a, the rest white
        assert_eq!(stock.surface, "#313131");

        let light = palette("#ffffff", "#123456", Some("#000000"));

        assert_eq!(light.rail, "#ffffff");
        assert_eq!(light.surface, "#d6d6d6");
        assert_eq!(light.text, "#000000");
        assert_eq!(light.accent, "#123456");
        assert_eq!(
            palette("red", "blue", Some("green")).accent,
            DEFAULT_ACCENT_COLOR
        );
    }
}
