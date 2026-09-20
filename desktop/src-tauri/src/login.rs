//! Signing in on the user's behalf.
//!
//! Shiver posts to the server's own `/login` and keeps the session token it returns, so the user
//! never meets the web login page. The password is sent straight to the server the user named and
//! is never written anywhere except the OS keychain (see `secrets`).

use std::time::Duration;

use serde_json::Value;
use zeroize::Zeroize;

use crate::error::{Error, Result};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Exchanges a username and password for a session token.
///
/// Refuses anything but https, even though `normalize_origin` has already refused it once. This is
/// the call that actually puts a password on the wire, and an entry stored by an older Shiver — from
/// before https was required — would otherwise still be signed in to over http.
pub async fn sign_in(origin: &str, identity: &str, password: &str) -> Result<String> {
    if !origin.starts_with("https://") {
        return Err(Error::Refused(
            "Shiver will not send a password over http. Use an https address for this server."
                .into(),
        ));
    }

    // A redirect is never followed on this call, and that is the whole point of saying so.
    // `reqwest` follows up to ten by default, and a 307 or 308 keeps the method *and the body* —
    // so a server answering `POST /login` with `Location: https://somewhere-else/` would have
    // Shiver hand the user's password to a host the user never named. Sharkord answers this
    // endpoint directly, so refusing to be sent elsewhere costs nothing legitimate. `http` is
    // where that policy and the body cap both live now.
    let client = shiver_core::http::client(LOGIN_TIMEOUT)
        .map_err(|error| Error::Unreachable(error.to_string()))?;

    // The serialised body is wiped once it has been handed over, rather than dropped with the
    // password still in it: a freed `String` keeps its bytes until something reuses that memory,
    // where a core dump or a debugger can read them.
    //
    // **This covers the copy Shiver makes, not every copy that exists.** `reqwest` buffers the body
    // again internally and that buffer is not reachable from here, so this narrows the window
    // rather than closing it. Worth doing for the one copy this code owns; not worth claiming more
    // than it does.
    let response = {
        let mut payload =
            serde_json::json!({ "identity": identity, "password": password }).to_string();

        let sending = client
            .post(format!("{origin}/login"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload.clone())
            .send();

        payload.zeroize();

        sending
            .await
            .map_err(|_| Error::Unreachable(origin.to_string()))?
    };

    let status = response.status();

    let body: Value = shiver_core::http::json_within_limit(response)
        .await
        .ok_or_else(|| Error::NotSharkord(origin.to_string()))?;

    if !status.is_success() {
        return Err(Error::Refused(login_error_message(&body)));
    }

    body.get("token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Refused("The server did not return a session".into()))
}

/// Sharkord reports a bad password as `{ errors: { identity: "Invalid credentials" } }` and other
/// failures as `{ error: "..." }`. Both are already written for a user to read.
fn login_error_message(body: &Value) -> String {
    if let Some(errors) = body.get("errors").and_then(Value::as_object) {
        if let Some(first) = errors.values().find_map(Value::as_str) {
            return presentable(first);
        }
    }

    body.get("error")
        .and_then(Value::as_str)
        .map(presentable)
        .unwrap_or_else(|| "Could not sign in".to_string())
}

/// The longest a server's own refusal may be before Shiver stops quoting it.
const MAX_SERVER_MESSAGE: usize = 200;

/// A server's words, made safe to put in Shiver's own panel.
///
/// These are passed through on the grounds that they are already written for a person to read.
/// That is true of Sharkord and not of whatever else may be answering on that address: this string
/// is rendered inside Shiver's own chrome, at a moment the user is already being asked about
/// credentials, which is a serviceable place to write "Your session expired, reset it at ...".
///
/// So: one line, no control characters, bounded, and nothing that reads as a link.
fn presentable(message: &str) -> String {
    let flattened: String = message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();

    let cleaned = flattened
        .split_whitespace()
        .filter(|word| {
            let word = word.to_ascii_lowercase();

            !word.contains("://") && !word.starts_with("www.")
        })
        .collect::<Vec<_>>()
        .join(" ");

    if cleaned.is_empty() {
        return "Could not sign in".to_string();
    }

    cleaned.chars().take(MAX_SERVER_MESSAGE).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refused before any network call, so a stored http entry cannot leak a password even though
    /// `normalize_origin` would no longer have let it be added.
    #[tokio::test]
    async fn refuses_to_send_a_password_over_http() {
        let refused = sign_in("http://chat.example.com", "someone", "hunter2").await;

        assert!(matches!(refused, Err(Error::Refused(_))));
        assert!(refused.unwrap_err().to_string().contains("https"));
    }
}
