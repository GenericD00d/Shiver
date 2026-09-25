//! Just enough JWT to read `exp`. Shiver never verifies tokens; the server does.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;

/// Renew when less than this is left, so a session never expires mid-use.
const REFRESH_MARGIN_SECS: u64 = 24 * 60 * 60;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Seconds until the token expires; `None` when it has no readable `exp` (treated as stale).
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

/// Worth replacing: unreadable, expired, or inside the refresh margin.
pub fn needs_refresh(token: &str) -> bool {
    seconds_left(token).map_or(true, |left| left < REFRESH_MARGIN_SECS)
}

/// Still usable right now, even if it is due for a refresh.
pub fn is_live(token: &str) -> bool {
    seconds_left(token).is_some_and(|left| left > 60)
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
        let week = token_expiring_at(now() + 7 * 24 * 60 * 60);
        let hour = token_expiring_at(now() + 60 * 60);
        let expired = token_expiring_at(now().saturating_sub(60));

        assert!(!needs_refresh(&week) && is_live(&week));
        assert!(
            needs_refresh(&hour) && is_live(&hour),
            "due for renewal but still usable"
        );
        assert!(needs_refresh(&expired) && !is_live(&expired));

        for junk in ["", "not-a-jwt", "header.!!!!.signature"] {
            assert!(needs_refresh(junk) && !is_live(junk));
        }
    }
}
