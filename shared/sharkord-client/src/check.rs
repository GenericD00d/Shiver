//! The add-server check both clients run before storing anything.

use std::{
    collections::hash_map::RandomState,
    hash::BuildHasher,
    sync::Mutex,
    time::{Duration, Instant},
};

use shiver_core::{
    login, normalize_origin,
    probe::{self, ServerInfo},
    LockExt,
};
use zeroize::Zeroizing;

const KEPT_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCheck {
    #[serde(flatten)]
    pub info: ServerInfo,
    /// Absent when unchecked (no credentials); `None` when checked and not installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<Option<String>>,
}

/// The session the last check signed in with, so adding that server does not sign in again.
#[derive(Default)]
pub struct CheckedSessions {
    keys: RandomState,
    held: Mutex<Option<(u64, Instant, Zeroizing<String>)>>,
}

impl CheckedSessions {
    fn keep(&self, origin: &str, identity: &str, password: &str, token: Zeroizing<String>) {
        *self.held.locked() = Some((
            self.keys.hash_one((origin, identity, password)),
            Instant::now(),
            token,
        ));
    }

    /// The kept session, once, if it was signed in with exactly these credentials recently.
    pub fn take(&self, origin: &str, identity: &str, password: &str) -> Option<Zeroizing<String>> {
        let (key, at, token) = self.held.locked().take()?;

        (key == self.keys.hash_one((origin, identity, password)) && at.elapsed() < KEPT_FOR)
            .then_some(token)
    }
}

/// `/info`, plus — given credentials — whether the companion plugin is installed, which only the
/// join payload of a signed-in connection reveals. The session is kept in `kept`, nothing is stored.
pub async fn check_server(
    origin: &str,
    identity: Option<&str>,
    password: Option<&str>,
    kept: &CheckedSessions,
) -> shiver_core::Result<ServerCheck> {
    let origin = normalize_origin(origin)?;
    let info = probe::fetch_info(&origin).await?;

    let (Some(identity), Some(password)) = (
        identity
            .map(str::trim)
            .filter(|identity| !identity.is_empty()),
        password.filter(|password| !password.is_empty()),
    ) else {
        return Ok(ServerCheck { info, plugin: None });
    };

    let token = Zeroizing::new(login::sign_in(&origin, identity, password).await?);
    let session = crate::open(&origin, &token, false)
        .await
        .map_err(|error| shiver_core::Error::Unreachable(format!("{origin}: {error}")))?;

    kept.keep(&origin, identity, password, token);

    Ok(ServerCheck {
        info,
        plugin: Some(session.joined.plugin_version.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_session_is_taken_once_and_only_with_the_same_credentials() {
        let kept = CheckedSessions::default();

        kept.keep("https://a", "me", "pw", Zeroizing::new("t".into()));
        assert!(kept.take("https://a", "me", "other").is_none());

        kept.keep("https://a", "me", "pw", Zeroizing::new("t".into()));
        assert_eq!(
            kept.take("https://a", "me", "pw")
                .as_deref()
                .map(String::as_str),
            Some("t")
        );
        assert!(kept.take("https://a", "me", "pw").is_none());
    }
}
