//! Registry pieces both clients store identically.

use serde::{Deserialize, Serialize};

/// Sharkord's own dark theme (`--background` and `--primary`), so a stock Shiver restyles nothing.
pub const DEFAULT_THEME_COLOR: &str = "#0a0a0a";
pub const DEFAULT_ACCENT_COLOR: &str = "#e5e5e5";

/// The longest account label kept; a longer one is cut, not refused.
const MAX_ACCOUNT_LABEL: usize = 64;

/// A typed account label: trimmed, invisible marks dropped and bounded; none when blank.
pub fn account_label(label: Option<&str>) -> Option<String> {
    let label = crate::text::clamp(label?.trim().to_string(), MAX_ACCOUNT_LABEL);

    (!label.is_empty()).then_some(label)
}

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

/// `#rrggbb` (lowercased), or `fallback`. Colours become CSS in every server page, so nothing else
/// may pass.
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

/// The colours the user picks, as a set: what Appearance edits and a saved preset holds. No text
/// colour picks dark or light text from the background; no accent text colour, the accent (made
/// readable where it is too close to the background); no sidebar colour, a shade off the background.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Colors {
    pub theme_color: String,
    pub accent_color: String,
    #[serde(default)]
    pub text_color: Option<String>,
    #[serde(default)]
    pub accent_text_color: Option<String>,
    /// the channel list, member list and top bars, and Shiver's rail
    #[serde(default)]
    pub sidebar_color: Option<String>,
}

impl Colors {
    pub fn sanitised(self) -> Self {
        Self {
            theme_color: sanitised_color(&self.theme_color, DEFAULT_THEME_COLOR),
            accent_color: sanitised_color(&self.accent_color, DEFAULT_ACCENT_COLOR),
            text_color: sanitised_optional_color(self.text_color.as_deref()),
            accent_text_color: sanitised_optional_color(self.accent_text_color.as_deref()),
            sidebar_color: sanitised_optional_color(self.sidebar_color.as_deref()),
        }
    }

    /// Whether these are Sharkord's own, in which case Shiver injects no css into pages.
    fn are_default(&self) -> bool {
        self.theme_color.eq_ignore_ascii_case(DEFAULT_THEME_COLOR)
            && self.accent_color.eq_ignore_ascii_case(DEFAULT_ACCENT_COLOR)
            && self.text_color.is_none()
            && self.accent_text_color.is_none()
            && self.sidebar_color.is_none()
    }
}

/// A set of colours the user saved under a name, to switch back to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ColorPreset {
    pub name: String,
    #[serde(flatten)]
    pub colors: Colors,
}

pub const MAX_COLOR_PRESETS: usize = 20;
const MAX_PRESET_NAME: usize = 32;

/// Presets as stored: named (trimmed, bounded; a blank one is dropped), one per name (the first),
/// colours sanitised, and no more than `MAX_COLOR_PRESETS`.
pub fn sanitised_presets(presets: Vec<ColorPreset>) -> Vec<ColorPreset> {
    let mut kept: Vec<ColorPreset> = Vec::new();

    for preset in presets {
        let name = crate::text::clamp(preset.name.trim().to_string(), MAX_PRESET_NAME);

        if name.is_empty() || kept.iter().any(|kept| kept.name == name) {
            continue;
        }

        kept.push(ColorPreset {
            name,
            colors: preset.colors.sanitised(),
        });

        if kept.len() == MAX_COLOR_PRESETS {
            break;
        }
    }

    kept
}

/// The red, green and blue of a `#rrggbb` colour.
pub fn rgb(value: &str) -> Option<[u8; 3]> {
    if !is_hex_color(value) {
        return None;
    }

    let channel = |at: usize| u8::from_str_radix(&value[at..at + 2], 16).ok();

    Some([channel(1)?, channel(3)?, channel(5)?])
}

/// The theme handed to server pages: `null` on Sharkord's own colours (pages are left untouched),
/// otherwise sanitised colours only, since the bridge turns them into CSS.
pub fn theme_payload(colors: &Colors) -> serde_json::Value {
    if colors.are_default() {
        return serde_json::Value::Null;
    }

    serde_json::to_value(colors.clone().sanitised()).unwrap_or_default()
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
        let colors = |theme: &str, accent: &str, accent_text: Option<&str>| Colors {
            theme_color: theme.into(),
            accent_color: accent.into(),
            text_color: None,
            accent_text_color: accent_text.map(Into::into),
            sidebar_color: None,
        };

        assert!(theme_payload(&colors(DEFAULT_THEME_COLOR, DEFAULT_ACCENT_COLOR, None)).is_null());

        let payload = theme_payload(&colors("#FFFFFF", "bad", None));

        assert_eq!(payload["accentColor"], DEFAULT_ACCENT_COLOR);
        assert_eq!(payload["themeColor"], "#ffffff");
        assert!(payload["accentTextColor"].is_null());

        let payload = theme_payload(&colors(
            DEFAULT_THEME_COLOR,
            DEFAULT_ACCENT_COLOR,
            Some("#AA0000"),
        ));

        assert_eq!(
            payload["accentTextColor"], "#aa0000",
            "an accent text colour alone restyles"
        );

        let sidebar = Colors {
            sidebar_color: Some("#121214".into()),
            ..colors(DEFAULT_THEME_COLOR, DEFAULT_ACCENT_COLOR, None)
        };

        assert_eq!(
            theme_payload(&sidebar)["sidebarColor"],
            "#121214",
            "so does a sidebar colour"
        );
    }

    #[test]
    fn presets_are_named_once_sanitised_and_bounded() {
        let preset = |name: &str, accent: &str| ColorPreset {
            name: name.into(),
            colors: Colors {
                theme_color: "#1F1F25".into(),
                accent_color: accent.into(),
                text_color: Some("not a colour".into()),
                accent_text_color: None,
                sidebar_color: Some("#12121".into()),
            },
        };

        let kept = sanitised_presets(vec![
            preset("  Cord  ", "#18181a"),
            preset("Cord", "#ffffff"),
            preset("   ", "#ffffff"),
            preset("Bad", "red; x"),
        ]);

        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].name, "Cord");
        assert_eq!(kept[0].colors.theme_color, "#1f1f25");
        assert_eq!(kept[0].colors.accent_color, "#18181a");
        assert_eq!(kept[0].colors.text_color, None);
        assert_eq!(kept[0].colors.sidebar_color, None);
        assert_eq!(kept[1].colors.accent_color, DEFAULT_ACCENT_COLOR);

        let many = (0..50).map(|n| preset(&n.to_string(), "#000000")).collect();

        assert_eq!(sanitised_presets(many).len(), MAX_COLOR_PRESETS);

        let read: ColorPreset = serde_json::from_str(
            r##"{"name":"Old","themeColor":"#000000","accentColor":"#ffffff"}"##,
        )
        .unwrap();

        assert_eq!(
            read.colors.accent_text_color, None,
            "a preset saved without the newer fields"
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
    fn an_account_label_is_trimmed_and_bounded() {
        assert_eq!(account_label(Some("  work  ")).as_deref(), Some("work"));
        assert_eq!(account_label(Some("   ")), None);
        assert_eq!(account_label(None), None);
        assert_eq!(
            account_label(Some(&"a".repeat(200))).map(|label| label.chars().count()),
            Some(MAX_ACCOUNT_LABEL + 1)
        );
    }
}
