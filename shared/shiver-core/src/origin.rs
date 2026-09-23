//! The origin rules. `is_same_origin` is the single comparison every webview boundary uses.

use url::Url;

use crate::error::{Error, Result};

/// Reduces a typed address to `https://host[:port]`. A bare host gets `https://`. Refuses any other
/// scheme (no exception for localhost or private addresses), credentials, and anything without a
/// host, since every boundary in Shiver is keyed on this string.
pub fn normalize_origin(input: &str) -> Result<String> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err(Error::InvalidOrigin("Enter a server address".into()));
    }

    let invalid = || Error::InvalidOrigin(format!("'{trimmed}' is not a valid address"));

    let url = match Url::parse(trimmed) {
        Ok(url) if url.has_host() => url,
        // no scheme (`chat.example.com`, `host:4991`) parses as relative or as a bogus scheme
        _ => Url::parse(&format!("https://{trimmed}")).map_err(|_| invalid())?,
    };

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
        .filter(|host| !host.is_empty())
        .ok_or_else(invalid)?;

    Ok(match url.port() {
        Some(port) => format!("https://{host}:{port}"),
        None => format!("https://{host}"),
    })
}

/// True when `url` is on exactly `origin` (scheme, host and port). Allocation-free.
pub fn is_same_origin(origin: &str, url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    let Some(rest) = origin
        .strip_prefix(url.scheme())
        .and_then(|rest| rest.strip_prefix("://"))
        .and_then(|rest| rest.strip_prefix(host))
    else {
        return false;
    };

    match url.port() {
        Some(port) => rest
            .strip_prefix(':')
            .is_some_and(|value| value.parse() == Ok(port)),
        None => rest.is_empty(),
    }
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
    fn a_bare_host_with_a_port_or_a_query_still_normalises() {
        assert_eq!(
            normalize_origin("localhost:4991").unwrap(),
            "https://localhost:4991"
        );
        assert_eq!(
            normalize_origin("chat.example.com/a?next=https://x").unwrap(),
            "https://chat.example.com"
        );
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
        assert!(!is_same_origin(
            origin,
            &Url::parse("https://example.com.evil.net").unwrap()
        ));
        assert!(is_same_origin(
            "https://example.com:8443",
            &Url::parse("https://example.com:8443/x").unwrap()
        ));
        assert!(!is_same_origin(
            "https://example.com:8443",
            &Url::parse("https://example.com:84").unwrap()
        ));
    }
}
