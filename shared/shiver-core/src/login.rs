//! Signing in to a Sharkord server with `POST /login`, on the user's behalf.

use std::time::Duration;

use serde_json::Value;

use crate::{
    error::{Error, Result},
    http,
};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);

/// The longest server message Shiver will quote back to the user.
const MAX_SERVER_MESSAGE: usize = 200;

/// `origin` plus the innermost cause of a transport failure (a certificate or DNS problem, say),
/// so "could not reach" says why.
pub fn unreachable(origin: &str, error: &(dyn std::error::Error + 'static)) -> Error {
    let mut cause = error;

    while let Some(inner) = cause.source() {
        cause = inner;
    }

    Error::Unreachable(format!("{origin} ({cause})"))
}

/// Exchanges a username and password for a session token. Refuses anything but https.
///
/// The request body is moved into the request rather than copied, so Shiver holds no second copy
/// of the password; reqwest's own buffer is outside its control.
pub async fn sign_in(origin: &str, identity: &str, password: &str) -> Result<String> {
    if !origin.starts_with("https://") {
        return Err(Error::Refused(
            "Shiver will not send a password over http. Use an https address for this server."
                .into(),
        ));
    }

    let body = serde_json::json!({ "identity": identity, "password": password }).to_string();

    let response = http::client()
        .post(format!("{origin}/login"))
        .timeout(LOGIN_TIMEOUT)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| unreachable(origin, &error))?;

    let status = response.status();
    let body = http::json_within_limit(response).await;

    if !status.is_success() {
        return Err(failure(origin, status, body));
    }

    body.as_ref()
        .and_then(|body| body.get("token"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::NotSharkord(origin.to_string()))
}

/// Only a 4xx (bar 408 and 429) refuses the credentials; anything else is the server's trouble.
fn failure(origin: &str, status: reqwest::StatusCode, body: Option<Value>) -> Error {
    if !status.is_client_error() || matches!(status.as_u16(), 408 | 429) {
        return Error::Unreachable(format!("{origin} (it answered {status})"));
    }

    Error::Refused(body.map_or_else(
        || format!("The server refused the sign-in ({status})"),
        |body| login_error_message(&body),
    ))
}

/// Sharkord's `{ errors: { field: msg } }` or `{ error: msg }`, made presentable.
pub fn login_error_message(body: &Value) -> String {
    body.get("errors")
        .and_then(Value::as_object)
        .and_then(|errors| errors.values().find_map(Value::as_str))
        .or_else(|| body.get("error").and_then(Value::as_str))
        .and_then(presentable)
        .unwrap_or_else(|| "Could not sign in".to_string())
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

    #[tokio::test]
    async fn refuses_to_send_a_password_over_http() {
        let refused = sign_in("http://chat.example.com", "someone", "hunter2").await;

        assert!(matches!(refused, Err(Error::Refused(message)) if message.contains("https")));
    }

    #[test]
    fn only_a_client_error_refuses_the_credentials() {
        use reqwest::StatusCode;

        for (status, refused) in [
            (StatusCode::UNAUTHORIZED, true),
            (StatusCode::BAD_REQUEST, true),
            (StatusCode::TOO_MANY_REQUESTS, false),
            (StatusCode::BAD_GATEWAY, false),
            (StatusCode::FOUND, false),
        ] {
            let error = failure("https://chat.example.com", status, None);

            assert_eq!(matches!(error, Error::Refused(_)), refused, "{status}");
        }
    }

    #[test]
    fn a_refusal_uses_the_servers_own_wording() {
        let body = serde_json::json!({ "errors": { "identity": "Invalid credentials" } });

        assert_eq!(login_error_message(&body), "Invalid credentials");
        assert_eq!(
            login_error_message(&serde_json::json!({ "error": "Server is full" })),
            "Server is full"
        );
        assert_eq!(
            login_error_message(&serde_json::json!({})),
            "Could not sign in"
        );
    }

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
}
