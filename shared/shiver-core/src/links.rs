//! Links a server's page asks Shiver to open in the browser. The page's word that the user clicked
//! is no proof (it runs in the page's own script world), so each client asks the user first, unless
//! they chose to trust that site.

use url::Url;

/// Sites the user may trust at most; trusting one more forgets the oldest.
const MAX_TRUSTED: usize = 200;
/// How much of an address the question shows.
const MAX_SHOWN: usize = 300;

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

/// Adds `site` to the trusted list, newest last.
pub fn trust(trusted: &mut Vec<String>, site: String) {
    trusted.retain(|known| known != &site);
    trusted.push(site);

    let excess = trusted.len().saturating_sub(MAX_TRUSTED);

    trusted.drain(..excess);
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

    #[test]
    fn trusting_is_bounded_and_moves_a_site_to_the_end() {
        let mut trusted = vec!["a".to_string(), "b".to_string()];

        trust(&mut trusted, "a".into());
        assert_eq!(trusted, ["b", "a"]);

        for index in 0..MAX_TRUSTED {
            trust(&mut trusted, index.to_string());
        }

        assert_eq!(trusted.len(), MAX_TRUSTED);
        assert_eq!(trusted[0], "0");
    }
}
