//! Links a server's page asks Shiver to open in the browser. The page's word that the user clicked
//! is no proof (it runs in the page's own script world), so each client asks the user first, unless
//! they chose to trust that site from that server: trusting a site from one server never lets
//! another open it unasked.
//!
//! The clients show the question in a native dialog with the buttons named here, and act on the
//! answer through [`Answer::from_choice`].

use serde::{Deserialize, Serialize};
use url::Url;

/// Sites the user may trust at most, across servers; trusting one more forgets the oldest.
const MAX_TRUSTED: usize = 200;
/// How much of an address the question shows.
const MAX_SHOWN: usize = 300;

pub const OPEN: &str = "Open";
pub const CANCEL: &str = "Cancel";

/// A site whose links open without asking, when a page of this rail entry asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedLink {
    pub entry_id: String,
    pub site: String,
}

/// What to do with a link a page asked to open.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// not a link Shiver opens (not http(s), or carrying credentials)
    Refuse,
    /// the user trusts this site from this server
    Open,
    /// put `question` to the user, with the buttons `OPEN`, `always` and `CANCEL`
    Ask {
        site: String,
        question: String,
        always: String,
    },
}

pub fn decide(trusted: &[TrustedLink], entry_id: &str, server: &str, url: &Url) -> Decision {
    let Some(site) = site(url) else {
        return Decision::Refuse;
    };

    if is_trusted(trusted, entry_id, &site) {
        return Decision::Open;
    }

    Decision::Ask {
        question: question(server, &site, url),
        always: always(&site),
        site,
    }
}

/// The user's answer to the question, from the button they chose.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Open,
    /// open it, and trust the site from this server from now on
    Always,
    Cancel,
}

impl Answer {
    pub fn from_choice(choice: &str, site: &str) -> Self {
        if choice == OPEN {
            Self::Open
        } else if choice == always(site) {
            Self::Always
        } else {
            Self::Cancel
        }
    }
}

/// The label of the button that trusts `site`.
pub fn always(site: &str) -> String {
    format!("Always for {site}")
}

/// The site a link goes to, as the browser will see it (lowercase, punycode), for http(s) only.
/// A link carrying credentials has none, so it is refused: `https://bank.example@evil.example/`
/// reads as one site and goes to another.
pub fn site(url: &Url) -> Option<String> {
    let plain = url.username().is_empty() && url.password().is_none();

    (plain && matches!(url.scheme(), "http" | "https"))
        .then(|| url.host_str())
        .flatten()
        .map(str::to_ascii_lowercase)
}

/// The question put to the user, naming the site first, where no length of address can hide it.
pub fn question(server: &str, site: &str, url: &Url) -> String {
    format!(
        "{server} wants to open a page on {site} in your browser:\n\n{}",
        crate::text::clamp(url.to_string(), MAX_SHOWN)
    )
}

pub fn is_trusted(trusted: &[TrustedLink], entry_id: &str, site: &str) -> bool {
    trusted
        .iter()
        .any(|link| link.entry_id == entry_id && link.site == site)
}

/// Trusts `site` from `entry_id`'s pages, newest last.
pub fn trust(trusted: &mut Vec<TrustedLink>, entry_id: &str, site: String) {
    trusted.retain(|link| !(link.entry_id == entry_id && link.site == site));
    trusted.push(TrustedLink {
        entry_id: entry_id.to_string(),
        site,
    });

    let excess = trusted.len().saturating_sub(MAX_TRUSTED);

    trusted.drain(..excess);
}

/// Forgets what was trusted from a rail entry that was removed.
pub fn forget_entry(trusted: &mut Vec<TrustedLink>, entry_id: &str) {
    trusted.retain(|link| link.entry_id != entry_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("a url")
    }

    #[test]
    fn a_site_is_the_host_the_browser_sees() {
        assert_eq!(
            site(&url("https://Example.COM/a")).as_deref(),
            Some("example.com")
        );
        assert_eq!(
            site(&url("https://bücher.example/")).as_deref(),
            Some("xn--bcher-kva.example")
        );
        assert_eq!(site(&url("mailto:someone@example.com")), None);
        assert_eq!(site(&url("https://bank.example@evil.example/")), None);
        assert_eq!(site(&url("https://:secret@evil.example/")), None);
    }

    #[test]
    fn the_question_names_the_site_before_the_address() {
        let long = url(&format!("https://evil.example/{}", "a".repeat(500)));

        assert!(question("Chat", "evil.example", &long)
            .starts_with("Chat wants to open a page on evil.example in your browser:"));
    }

    fn sites(trusted: &[TrustedLink]) -> Vec<(&str, &str)> {
        trusted
            .iter()
            .map(|link| (link.entry_id.as_str(), link.site.as_str()))
            .collect()
    }

    #[test]
    fn trusting_is_per_server_bounded_and_moves_a_site_to_the_end() {
        let mut trusted = Vec::new();

        trust(&mut trusted, "one", "a".into());
        trust(&mut trusted, "one", "b".into());
        trust(&mut trusted, "two", "a".into());
        trust(&mut trusted, "one", "a".into());
        assert_eq!(sites(&trusted), [("one", "b"), ("two", "a"), ("one", "a")]);

        assert!(is_trusted(&trusted, "one", "b"));
        assert!(
            !is_trusted(&trusted, "two", "b"),
            "trusted from another server"
        );

        forget_entry(&mut trusted, "one");
        assert_eq!(sites(&trusted), [("two", "a")]);

        for index in 0..MAX_TRUSTED {
            trust(&mut trusted, "two", index.to_string());
        }

        assert_eq!(trusted.len(), MAX_TRUSTED);
        assert_eq!(trusted[0].site, "0");
    }

    #[test]
    fn a_link_opens_asks_or_is_refused() {
        let trusted = vec![TrustedLink {
            entry_id: "one".into(),
            site: "example.com".into(),
        }];
        let link = url("https://example.com/a");

        assert_eq!(decide(&trusted, "one", "Chat", &link), Decision::Open);
        assert_eq!(
            decide(&trusted, "one", "Chat", &url("javascript:alert(1)")),
            Decision::Refuse
        );

        let Decision::Ask {
            site,
            question,
            always: label,
        } = decide(&trusted, "two", "Other", &link)
        else {
            panic!("another server's page is asked about");
        };

        assert_eq!(site, "example.com");
        assert!(question.starts_with("Other wants to open a page on example.com"));
        assert_eq!(Answer::from_choice(&label, &site), Answer::Always);
        assert_eq!(Answer::from_choice(OPEN, &site), Answer::Open);
        assert_eq!(Answer::from_choice(CANCEL, &site), Answer::Cancel);
        assert_eq!(
            Answer::from_choice(&always("evil.example"), &site),
            Answer::Cancel
        );
    }
}
