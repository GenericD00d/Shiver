//! Just enough JWT to read `exp`. Shiver never verifies tokens; the server does.

use std::time::{SystemTime, UNIX_EPOCH};

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
    let payload = base64url_decode(token.split('.').nth(1)?)?;
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

/// base64url without padding, as JWT payloads are.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32)
    };

    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for byte in input.bytes().take_while(|byte| *byte != b'=') {
        buffer = (buffer << 6) | value(byte)?;
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `header.<base64url({"exp":exp})>.signature`
    fn token_expiring_at(exp: u64) -> String {
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut encoded = String::new();

        for chunk in format!(r#"{{"exp":{exp}}}"#).as_bytes().chunks(3) {
            let buffer = chunk
                .iter()
                .enumerate()
                .fold(0u32, |buffer, (index, byte)| {
                    buffer | (*byte as u32) << (16 - 8 * index)
                });

            for index in 0..=chunk.len() {
                encoded.push(alphabet[((buffer >> (18 - 6 * index)) & 0x3f) as usize] as char);
            }
        }

        format!("header.{encoded}.signature")
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
