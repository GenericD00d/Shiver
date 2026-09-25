//! Words from a server, made fit for Shiver's own panels and notifications.

/// The longest server message Shiver will quote back to the user.
const MAX_SERVER_MESSAGE: usize = 200;

/// Drops invisible characters (bidi overrides, zero-width marks) a server could disguise text
/// with, then shortens by characters, marking the cut with an ellipsis.
pub fn clamp(text: String, limit: usize) -> String {
    let text: String = text
        .chars()
        .filter(|character| !is_invisible(*character))
        .collect();

    match text.char_indices().nth(limit) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

/// A server's words made safe for Shiver's own panels: one line, no control characters, no links,
/// no bidi tricks, bounded; `None` when nothing is left.
pub fn presentable(message: &str) -> Option<String> {
    let cleaned = message
        .chars()
        .filter(|character| !is_invisible(*character))
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .filter(|word| !looks_like_link(word))
        .collect::<Vec<_>>()
        .join(" ");

    (!cleaned.is_empty()).then(|| cleaned.chars().take(MAX_SERVER_MESSAGE).collect())
}

fn looks_like_link(word: &str) -> bool {
    let word = word.to_ascii_lowercase();
    let word = word.trim_matches(|character: char| !character.is_alphanumeric());

    word.contains("://")
        || word.starts_with("www.")
        // a bare domain with a path: `evil.example/reset`
        || word.split('/').next().is_some_and(|host| host.contains('.') && word.contains('/'))
}

fn is_invisible(character: char) -> bool {
    matches!(character, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_message_cannot_carry_links_or_tricks() {
        assert_eq!(
            presentable(
                "Session expired,\nreset it at https://evil.example/x or evil.example/reset now"
            )
            .as_deref(),
            Some("Session expired, reset it at or now")
        );
        assert_eq!(presentable("ad\u{202e}min").as_deref(), Some("admin"));
        assert_eq!(
            presentable(&"x".repeat(500)).map(|text| text.len()),
            Some(MAX_SERVER_MESSAGE)
        );
        assert_eq!(presentable("https://only.a.link"), None);
    }

    #[test]
    fn clamping_drops_invisible_marks_and_counts_characters() {
        assert_eq!(clamp("héllo".into(), 2), "hé…");
        assert_eq!(clamp("hi".into(), 2), "hi");
        assert_eq!(clamp("ad\u{202e}n\u{200b}imda".into(), 5), "adnim…");
    }
}
