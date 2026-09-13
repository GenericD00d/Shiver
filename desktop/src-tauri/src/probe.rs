use std::time::Duration;

use serde_json::Value;

use crate::{
    error::{Error, Result},
    model::ServerInfo,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Reads a server's public identity from `GET /info`.
///
/// This runs in rust rather than in a webview on purpose: it needs no session, it is not subject
/// to CORS, and it must not run inside another server's page.
pub async fn fetch_info(origin: &str) -> Result<ServerInfo> {
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        // Nowhere but the address the user typed. `reqwest` follows redirects by default, which
        // would let a server point Shiver's own request at anything — a host on the user's network,
        // or a third party that would learn the device's address from being contacted.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Shiver/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| Error::Unreachable(error.to_string()))?;

    let response = client
        .get(format!("{origin}/info"))
        .send()
        .await
        .map_err(|_| Error::Unreachable(origin.to_string()))?;

    if !response.status().is_success() {
        return Err(Error::NotSharkord(origin.to_string()));
    }

    let body: Value = response
        .json()
        .await
        .map_err(|_| Error::NotSharkord(origin.to_string()))?;

    // serverId and name are the two fields every Sharkord /info returns, so their absence is how
    // Shiver tells a Sharkord server from any other host that happens to answer on /info
    let server_id = body
        .get("serverId")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::NotSharkord(origin.to_string()))?;

    let name = body
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::NotSharkord(origin.to_string()))?;

    Ok(ServerInfo {
        origin: origin.to_string(),
        server_id: server_id.to_string(),
        name: name.to_string(),
        description: body
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        icon_url: logo_url(origin, &body),
    })
}

/// The logo arrives as the server's file row, which is served from /public by its stored name.
///
/// Built through `Url` rather than `format!`, because the name is the server's own string. Formatted
/// in, a name containing `?` or `#` would end the path and turn the rest into a query or a fragment
/// — pointing Shiver's request at something other than the file it meant to ask for. `path_segments_mut`
/// percent-encodes each segment, so the name can only ever be a name. It stays on the origin either
/// way, which is why this was a nit rather than a hole.
fn logo_url(origin: &str, body: &Value) -> Option<String> {
    let name = body.get("logo")?.get("name")?.as_str()?;

    if name.is_empty() {
        return None;
    }

    let mut url = url::Url::parse(origin).ok()?;

    url.path_segments_mut().ok()?.extend(["public", name]);

    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logo(name: &str) -> Option<String> {
        logo_url(
            "https://chat.example.com",
            &serde_json::json!({ "logo": { "name": name } }),
        )
    }

    #[test]
    fn an_ordinary_logo_name_reads_as_it_did_before() {
        assert_eq!(
            logo("server-logo.png").as_deref(),
            Some("https://chat.example.com/public/server-logo.png")
        );
    }

    /// The reason this stopped being a `format!`. A name ending the path would have pointed the
    /// request at `/public/` with everything after `?` as a query.
    #[test]
    fn a_name_cannot_end_the_path_and_start_a_query() {
        let url = logo("x.png?take=/etc/passwd").expect("a name is still a name");

        assert!(url.starts_with("https://chat.example.com/public/"));
        assert!(!url.contains('?'));
    }

    /// Nor a fragment, which would have cut the path short instead.
    #[test]
    fn a_name_cannot_start_a_fragment() {
        let url = logo("x.png#anchor").expect("a name is still a name");

        assert!(url.starts_with("https://chat.example.com/public/"));
        assert!(!url.contains('#'));
    }

    /// And it stays one segment, so it cannot add a path of its own.
    #[test]
    fn a_name_cannot_add_path_segments() {
        let url = logo("../../secret").expect("a name is still a name");

        assert!(url.starts_with("https://chat.example.com/public/"));
        assert!(!url.contains("/../"));
    }

    /// A server with no logo keeps its initials rather than a url to nothing.
    #[test]
    fn no_logo_is_no_url() {
        assert_eq!(logo(""), None);
        assert_eq!(
            logo_url("https://chat.example.com", &serde_json::json!({})),
            None
        );
    }
}
