//! Signing in on the user's behalf.
//!
//! The same `POST /login` the desktop client uses, and the same shape of answer. It exists here now
//! because there is finally somewhere to keep what comes back: before the encrypted store, mobile
//! had nowhere to put a session and so had nothing to do with a password.
//!
//! What differs from desktop is what Shiver keeps afterwards. Desktop stores the password too, so it
//! can sign in again when a session expires. This keeps only the token — enough to open the server
//! signed in and to let the core watch it, and one less secret held on a device that travels.

use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Exchanges a username and password for a session token.
///
/// Refuses anything but https before it sends, the same as desktop: this is the call that puts a
/// password on the wire, and `normalize_origin` having already refused http is not a reason for the
/// one place it matters most to take that on trust.
pub async fn sign_in(origin: &str, identity: &str, password: &str) -> Result<String> {
    if !origin.starts_with("https://") {
        return Err(Error::Refused(
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

    /// Refused before any network call, so a password cannot leave the device in the clear even if
    /// an http origin reached this far.
    #[tokio::test]
    async fn refuses_to_send_a_password_over_http() {
        let refused = sign_in("http://chat.example.com", "someone", "hunter2").await;

        assert!(matches!(refused, Err(Error::Refused(_))));
        assert!(refused.unwrap_err().to_string().contains("https"));
    }

    #[test]
    fn a_refusal_uses_the_servers_own_wording() {
        let body = serde_json::json!({ "errors": { "identity": "Invalid credentials" } });

        assert_eq!(login_error_message(&body), "Invalid credentials");

        let other = serde_json::json!({ "error": "Server is full" });

        assert_eq!(login_error_message(&other), "Server is full");
        assert_eq!(
            login_error_message(&serde_json::json!({})),
            "Could not sign in"
        );
    }
}
