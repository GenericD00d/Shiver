//! Signing in on the user's behalf.
//!
//! Shiver posts to the server's own `/login` and keeps the session token it returns, so the user
//! never meets the web login page. The password is sent straight to the server the user named and
//! is never written anywhere except the OS keychain (see `secrets`).

use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Exchanges a username and password for a session token.
///
/// Refuses anything but https, even though `normalize_origin` has already refused it once. This is
/// the call that actually puts a password on the wire, and an entry stored by an older Shiver — from
/// before https was required — would otherwise still be signed in to over http.
pub async fn sign_in(origin: &str, identity: &str, password: &str) -> Result<String> {
    if !origin.starts_with("https://") {
        return Err(Error::SignIn(
            "Shiver will not send a password over http. Use an https address for this server.".into(),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(LOGIN_TIMEOUT)
        // A redirect is never followed on this call, and that is the whole point of saying so.
        // `reqwest` follows up to ten by default, and a 307 or 308 keeps the method *and the body* —
        // so a server answering `POST /login` with `Location: https://somewhere-else/` would have
        // Shiver hand the user's password to a host the user never named. Sharkord answers this
        // endpoint directly, so refusing to be sent elsewhere costs nothing legitimate.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Shiver/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| Error::Unreachable(error.to_string()))?;

    let response = client
        .post(format!("{origin}/login"))
        .json(&serde_json::json!({ "identity": identity, "password": password }))
        .send()
        .await
        .map_err(|_| Error::Unreachable(origin.to_string()))?;

    let status = response.status();

    let body: Value = response
        .json()
        .await
        .map_err(|_| Error::NotSharkord(origin.to_string()))?;

    if !status.is_success() {
        return Err(Error::SignIn(login_error_message(&body)));
    }

    body.get("token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::SignIn("The server did not return a session".into()))
}

/// Sharkord reports a bad password as `{ errors: { identity: "Invalid credentials" } }` and other
/// failures as `{ error: "..." }`. Both are already written for a user to read.
fn login_error_message(body: &Value) -> String {
    if let Some(errors) = body.get("errors").and_then(Value::as_object) {
        if let Some(first) = errors.values().find_map(Value::as_str) {
            return first.to_string();
        }
    }

    body.get("error")
        .and_then(Value::as_str)
        .unwrap_or("Could not sign in")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refused before any network call, so a stored http entry cannot leak a password even though
    /// `normalize_origin` would no longer have let it be added.
    #[tokio::test]
    async fn refuses_to_send_a_password_over_http() {
        let refused = sign_in("http://chat.example.com", "someone", "hunter2").await;

        assert!(matches!(refused, Err(Error::SignIn(_))));
        assert!(refused.unwrap_err().to_string().contains("https"));
    }
}
