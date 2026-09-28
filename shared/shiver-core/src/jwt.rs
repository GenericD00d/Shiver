//! Just enough JWT to read `exp`, so both clients renew a session before the server refuses it.
//! Shiver never verifies tokens; the server does.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;

use crate::LockExt;

/// Renew when less than this is left, so a session never expires mid-use.
const REFRESH_MARGIN_SECS: u64 = 24 * 60 * 60;

/// Less than this left and a token is not worth opening a page or a socket with.
const LIVE_MARGIN_SECS: u64 = 60;

/// Minimum spacing between background renewals of one entry's session, so a server that is down
/// is not asked again on every page opened or socket reconnected.
const RENEW_AFTER: Duration = Duration::from_secs(5 * 60);

/// How usable a session token is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// good for more than a day
    Fresh,
    /// still usable, but due for renewal: use it, and renew in the background
    Due,
    /// expired, about to, or unreadable (no `exp`): renew before using it
    Expired,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Seconds until the token expires; `None` when it has no readable `exp` (treated as expired).
fn seconds_left(token: &str) -> Option<u64> {
    let payload = URL_SAFE_NO_PAD
        .decode(token.split('.').nth(1)?.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&payload).ok()?;
    let exp = claims.get("exp")?;
    let exp = exp
        .as_u64()
        .or_else(|| exp.as_f64().filter(|exp| *exp >= 0.0).map(|exp| exp as u64))?;

    Some(exp.saturating_sub(now()))
}

pub fn freshness(token: &str) -> Freshness {
    match seconds_left(token) {
        Some(left) if left >= REFRESH_MARGIN_SECS => Freshness::Fresh,
        Some(left) if left > LIVE_MARGIN_SECS => Freshness::Due,
        _ => Freshness::Expired,
    }
}

/// Whether `token` expires later than `other` (a token without a readable `exp` never does).
pub fn outlasts(token: &str, other: &str) -> bool {
    seconds_left(token) > seconds_left(other)
}

/// When each entry's last background renewal started; cleared when one succeeds.
#[derive(Default)]
pub struct Renewals(Mutex<HashMap<String, Instant>>);

impl Renewals {
    /// Claims a background renewal for `entry_id`, unless one started less than five minutes ago
    /// (it is still running, or failed a moment ago).
    pub fn begin(&self, entry_id: &str) -> bool {
        let mut renewals = self.0.locked();

        if renewals
            .get(entry_id)
            .is_some_and(|started| started.elapsed() < RENEW_AFTER)
        {
            return false;
        }

        renewals.insert(entry_id.to_string(), Instant::now());

        true
    }

    /// The renewal worked: the next one may start whenever it is due.
    pub fn succeeded(&self, entry_id: &str) {
        self.0.locked().remove(entry_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `header.<base64url({"exp":exp})>.signature`
    fn token_expiring_at(exp: u64) -> String {
        let claims = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));

        format!("header.{claims}.signature")
    }

    #[test]
    fn freshness_is_read_from_exp() {
        assert_eq!(
            freshness(&token_expiring_at(now() + 7 * 24 * 60 * 60)),
            Freshness::Fresh
        );
        assert_eq!(
            freshness(&token_expiring_at(now() + 60 * 60)),
            Freshness::Due,
            "due for renewal but still usable"
        );
        assert_eq!(
            freshness(&token_expiring_at(now() + 30)),
            Freshness::Expired
        );
        assert_eq!(
            freshness(&token_expiring_at(now().saturating_sub(60))),
            Freshness::Expired
        );

        for junk in ["", "not-a-jwt", "header.!!!!.signature"] {
            assert_eq!(freshness(junk), Freshness::Expired, "{junk:?}");
        }
    }

    #[test]
    fn the_later_expiry_outlasts() {
        let week = token_expiring_at(now() + 7 * 24 * 60 * 60);
        let hour = token_expiring_at(now() + 60 * 60);

        assert!(outlasts(&week, &hour));
        assert!(!outlasts(&hour, &week));
        assert!(!outlasts(&hour, &hour));
        assert!(outlasts(&hour, "not-a-jwt"));
        assert!(!outlasts("not-a-jwt", &hour));
    }

    #[test]
    fn background_renewals_are_spaced_until_one_succeeds() {
        let renewals = Renewals::default();

        assert!(renewals.begin("a"));
        assert!(!renewals.begin("a"), "running, or failed a moment ago");
        assert!(renewals.begin("b"), "per entry");

        renewals.succeeded("a");
        assert!(renewals.begin("a"));
    }
}
