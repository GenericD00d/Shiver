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

/// The largest logo Shiver will inline.
///
/// The rail's icons ride along in the bridge payload, which is a string evaluated in the page, so a
/// server with a multi-megabyte logo would make every server open slowly. Past this the entry keeps
/// its initials, which is a smaller loss than a slow client.
const MAX_ICON_BYTES: usize = 256 * 1024;

/// Fetches a server logo and returns it as a `data:` uri.
///
/// Downloaded here rather than linked in the page for the reason on `ServerEntry::icon_data`: the
/// rail is drawn inside a server's own page, and a linked icon would tell that page where every
/// other server lives. `None` on any failure — an icon is decoration, and a server whose logo is
/// unreachable still belongs in the rail.
pub async fn fetch_icon(url: &str) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        // Same rule as the probe above, and it matters more here: this url is built from a file
        // name the server chose, and a redirect is the one way that name could reach off the
        // server's own host. An icon is decoration — a logo that will not load leaves initials.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Shiver/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;

    let response = client.get(url).send().await.ok()?;

    if !response.status().is_success() {
        return None;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
        .unwrap_or_else(|| "image/png".to_string());

    // whatever answered is about to be inlined into a page as an image, so only take it if the
    // server says it is one
    if !content_type.starts_with("image/") {
        return None;
    }

    let bytes = response.bytes().await.ok()?;

    if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES {
        return None;
    }

    Some(format!(
        "data:{content_type};base64,{}",
        STANDARD.encode(&bytes)
    ))
}

/// The logo arrives as the server's file row, which is served from /public by its stored name.
fn logo_url(origin: &str, body: &Value) -> Option<String> {
    let name = body.get("logo")?.get("name")?.as_str()?;

    if name.is_empty() {
        return None;
    }

    // Built through `Url` rather than `format!`: the name is the server's own string, and formatted
    // in, one containing `?` or `#` would end the path and turn the rest into a query or a fragment.
    // `path_segments_mut` percent-encodes each segment, so a name can only ever be a name. Same
    // change as desktop's `probe.rs`, where the reasoning is written out.
    let mut url = url::Url::parse(origin).ok()?;

    url.path_segments_mut().ok()?.extend(["public", name]);

    Some(url.into())
}
