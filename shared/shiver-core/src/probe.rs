//! Reading a server's public identity (`GET /info`) and its logo, before any sign-in.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::{Error, Result},
    http,
    login::unreachable,
    text::presentable,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

const MAX_NAME: usize = 64;

/// The largest logo Shiver will inline as a `data:` uri.
const MAX_ICON_BYTES: usize = 256 * 1024;

/// What `GET /info` tells a client that has not signed in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub origin: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
}

/// Fetches `/info`. `serverId` and `name` are required: their absence means "not Sharkord".
pub async fn fetch_info(origin: &str) -> Result<ServerInfo> {
    let response = http::client()
        .get(format!("{origin}/info"))
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|error| unreachable(origin, &error))?;

    let body = match response.status().is_success() {
        true => http::json_within_limit(response).await,
        false => None,
    };

    body.and_then(|body| parse_info(origin, &body))
        .ok_or_else(|| Error::NotSharkord(origin.to_string()))
}

/// `serverId` and `name` are required. The words are cleaned and bounded: they are shown in the
/// rail (other servers' pages included), notifications and menus.
fn parse_info(origin: &str, body: &Value) -> Option<ServerInfo> {
    let text = |key: &str| body.get(key).and_then(Value::as_str);

    text("serverId")?;

    Some(ServerInfo {
        origin: origin.to_string(),
        name: presentable(text("name")?).map_or_else(
            || origin.trim_start_matches("https://").to_string(),
            |name| name.chars().take(MAX_NAME).collect(),
        ),
        description: text("description").and_then(presentable),
        icon_url: logo_url(origin, body),
    })
}

/// The logo's url on the server's own origin. Built with `Url` so a stored name containing `?`,
/// `#` or `/` stays one path segment.
fn logo_url(origin: &str, body: &Value) -> Option<String> {
    let name = body.get("logo")?.get("name")?.as_str()?;

    if name.is_empty() || name == "." || name == ".." {
        return None;
    }

    let mut url = url::Url::parse(origin).ok()?;

    url.path_segments_mut().ok()?.extend(["public", name]);

    Some(url.into())
}

/// Downloads a logo as a `data:` uri, or `None` on any failure. Only images, at most
/// `MAX_ICON_BYTES`, read incrementally so an oversized answer is never buffered.
pub async fn fetch_icon(url: &str) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let response = http::client()
        .get(url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)?
        .to_str()
        .ok()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();

    // a plain media type only, so nothing else can ride into the data uri
    let valid = content_type.starts_with("image/")
        && content_type
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/+.-".contains(&byte));

    if !valid {
        return None;
    }

    let bytes = http::bytes_within_limit(response, MAX_ICON_BYTES).await?;

    (!bytes.is_empty()).then(|| format!("data:{content_type};base64,{}", STANDARD.encode(bytes)))
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
    fn an_ordinary_logo_name_is_under_public() {
        assert_eq!(
            logo("server-logo.png").as_deref(),
            Some("https://chat.example.com/public/server-logo.png")
        );
    }

    #[test]
    fn a_name_stays_one_segment() {
        for name in ["x.png?take=/etc/passwd", "x.png#anchor", "../../secret"] {
            let url = logo(name).expect("a name is still a name");

            assert!(url.starts_with("https://chat.example.com/public/"));
            assert!(
                !url.contains('?') && !url.contains('#') && !url.contains("/../"),
                "{url}"
            );
        }
    }

    #[test]
    fn info_words_are_cleaned_and_bounded() {
        let info = |body| parse_info("https://chat.example.com", &body);
        let long = "x".repeat(500);
        let parsed = info(serde_json::json!({
            "serverId": "a", "name": "Chat\u{202e}\n room", "description": "see https://evil.example"
        }))
        .unwrap();

        assert_eq!(parsed.name, "Chat room");
        assert_eq!(parsed.description.as_deref(), Some("see"));
        assert_eq!(
            info(serde_json::json!({ "serverId": "a", "name": long })).map(|info| info.name.len()),
            Some(MAX_NAME)
        );
        assert_eq!(
            info(serde_json::json!({ "serverId": "a", "name": "\u{200b}" })).map(|info| info.name),
            Some("chat.example.com".into())
        );
        assert_eq!(info(serde_json::json!({ "name": "Chat" })), None);
    }

    #[test]
    fn no_logo_or_a_dot_name_is_no_url() {
        assert_eq!(logo(""), None);
        assert_eq!(logo(".."), None);
        assert_eq!(
            logo_url("https://chat.example.com", &serde_json::json!({})),
            None
        );
    }
}
