//! Sharkord's message html, read for what a notification needs: its text and who it mentions.

/// Mentioned users won't be many; a message listing more is not read further.
const MAX_MENTIONS: usize = 100;

/// The users a message's html mentions: `<span>`s carrying both `data-type="mention"` and
/// `data-user-id="<id>"`, in either order (Sharkord's own `hasMention` reads them the same way).
pub(crate) fn mentioned(html: &str) -> Vec<i64> {
    let attribute = |tag: &str, name: &str| -> Option<String> {
        tag.match_indices(name).find_map(|(at, _)| {
            if !tag[..at].ends_with(char::is_whitespace) {
                return None;
            }

            let value = tag[at + name.len()..].strip_prefix("=\"")?;

            Some(value[..value.find('"')?].to_string())
        })
    };

    let mut close = NextClose::default();

    html.match_indices("<span")
        .filter_map(|(at, _)| {
            let tag = &html[at..close.at_or_after(html, at)?];

            (attribute(tag, "data-type").as_deref() == Some("mention"))
                .then(|| attribute(tag, "data-user-id")?.parse().ok())
                .flatten()
        })
        .take(MAX_MENTIONS)
        .collect()
}

/// The next `>` in a text read front to back, looked for once and reused until the reader passes it,
/// so a message of `<a<a<a…` with no `>` is scanned once rather than once per `<`.
#[derive(Default)]
struct NextClose(Option<Option<usize>>);

impl NextClose {
    /// The index of the first `>` in `text` at or after `from`; `from` never goes backwards.
    fn at_or_after(&mut self, text: &str, from: usize) -> Option<usize> {
        let found = match self.0 {
            Some(Some(end)) if end >= from => Some(end),
            Some(None) => None,
            _ => text[from..].find('>').map(|offset| from + offset),
        };

        self.0 = Some(found);

        found
    }
}

/// Sharkord's message html as one line of text for a notification. Tags become word breaks, common
/// entities are decoded (named and numeric) and whitespace collapses. Not a sanitiser: the result
/// is never put back into html.
pub(crate) fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut pending_space = false;
    let mut rest = html;

    let mut close = NextClose::default();

    let push = |text: &mut String, pending_space: &mut bool, piece: &str| {
        if *pending_space && !text.is_empty() {
            text.push(' ');
        }

        *pending_space = false;
        text.push_str(piece);
    };

    while let Some(character) = rest.chars().next() {
        let after = &rest[character.len_utf8()..];

        match character {
            // only a real tag: `<` followed by a letter, `/` or `!`, and closed by a `>`
            '<' if after.starts_with(|next: char| {
                next.is_ascii_alphabetic() || next == '/' || next == '!'
            }) =>
            {
                if let Some(end) = close.at_or_after(html, html.len() - after.len()) {
                    rest = &html[end + 1..];
                    pending_space = true;

                    continue;
                }

                push(&mut text, &mut pending_space, "<");
            }
            '&' => {
                if let Some((decoded, length)) = entity(after) {
                    match decoded {
                        ' ' => pending_space = true,
                        other => push(
                            &mut text,
                            &mut pending_space,
                            other.encode_utf8(&mut [0; 4]),
                        ),
                    }

                    rest = &after[length..];

                    continue;
                }

                push(&mut text, &mut pending_space, "&");
            }
            _ if character.is_whitespace() => pending_space = true,
            _ => push(
                &mut text,
                &mut pending_space,
                character.encode_utf8(&mut [0; 4]),
            ),
        }

        rest = after;
    }

    text
}

/// Decodes an entity at the start of `text` (after its `&`): the character and how many bytes it
/// used, including the `;`.
fn entity(text: &str) -> Option<(char, usize)> {
    let end = text
        .char_indices()
        .take(12)
        .find(|(_, character)| *character == ';')?
        .0;
    let name = &text[..end];

    let decoded = match name {
        "nbsp" => ' ',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "amp" => '&',
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };

            char::from_u32(code).filter(|character| !character.is_control())?
        }
    };

    Some((decoded, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Joined, NewMessage};

    /// Sharkord's own `hasMention` cases.
    #[test]
    fn mentions_are_read_as_sharkord_reads_them() {
        let one = |html: &str| mentioned(html);

        assert_eq!(
            one(r#"<p>Hi <span data-type="mention" data-user-id="123">@a</span></p>"#),
            [123]
        );
        assert_eq!(
            one(
                r#"<p><span class="mention" data-user-id="12" data-type="mention">@a</span> and <span data-type="mention" data-user-id="456">@b</span></p>"#
            ),
            [12, 456]
        );

        for html in [
            "<p>Hello world</p>",
            r#"<span data-type="mention">@a</span>"#,
            r#"<span data-type="mention" data-user-id="abc">@a</span>"#,
            r#"<span data-type="link" data-user-id="1">x</span>"#,
            r#"<span xdata-type="mention" data-user-id="1">x</span>"#,
            r#"<span data-typex="y" data-type="link" data-user-id="1">x</span>"#,
            r#"<p data-type="mention" data-user-id="1">x</p>"#,
            r#"<span data-type="mention" data-user-id="1""#,
        ] {
            assert!(one(html).is_empty(), "{html}");
        }

        let joined = Joined {
            own_user_id: Some(123),
            ..Joined::default()
        };
        let message = NewMessage {
            channel_id: 1,
            user_id: Some(9),
            plugin_id: None,
            text: String::new(),
            mentioned: vec![1234, 123],
        };

        assert!(message.mentions_me(&joined));
        assert!(!NewMessage {
            mentioned: vec![12],
            ..message
        }
        .mentions_me(&joined));
    }

    #[test]
    fn plain_text_strips_tags_and_keeps_less_than_signs() {
        assert_eq!(plain_text("1 < 2 and 3 > 4"), "1 < 2 and 3 > 4");
        assert_eq!(plain_text("5<6"), "5<6");
        // `<` + letter with no closing `>` is text, not the start of a tag that eats the rest
        assert_eq!(plain_text("i think a <b and c"), "i think a <b and c");
        assert_eq!(plain_text("<p>hello</p><p>there</p>"), "hello there");
        assert_eq!(plain_text("<img src=\"x\">caption"), "caption");
        assert_eq!(plain_text("<!-- note -->text"), "text");
        assert_eq!(plain_text("  spaced   out  "), "spaced out");
        assert_eq!(plain_text("<p></p>"), "");
    }

    #[test]
    fn unclosed_tags_are_read_in_one_pass() {
        // a message long past Sharkord's limit, so a rescan per `<` would show
        let hostile = "<a".repeat(200_000);
        let started = std::time::Instant::now();

        assert_eq!(plain_text(&hostile), hostile);
        assert!(mentioned(&"<span".repeat(200_000)).is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));

        // a later `>` still closes the tags before it, and the text after it survives
        assert_eq!(plain_text("<a<b>x</b>"), "x");
    }

    #[test]
    fn entities_are_decoded_once() {
        assert_eq!(plain_text("&lt;tag&gt;"), "<tag>");
        assert_eq!(plain_text("a&nbsp;b"), "a b");
        assert_eq!(
            plain_text("it&#39;s &#x2019; &quot;q&quot;"),
            "it's \u{2019} \"q\""
        );
        assert_eq!(plain_text("&amp;lt;"), "&lt;");
        assert_eq!(plain_text("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(plain_text("&unknown;"), "&unknown;");
        assert_eq!(plain_text("&#0; &#x7;"), "&#0; &#x7;");
    }
}
