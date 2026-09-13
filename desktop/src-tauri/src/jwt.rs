//! Just enough JWT to know when a session is about to run out.
//!
//! Shiver never verifies these tokens, that is the server's job. It only reads the `exp` claim so it
//! can sign in again *before* a stale token drops the user onto the login page.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Re-authenticate when less than this is left, so a long session never expires mid-use.
const REFRESH_MARGIN_SECS: u64 = 24 * 60 * 60;

/// True when the token is missing, unreadable, expired, or close enough to expiry to be worth
/// replacing. Anything Shiver cannot parse counts as stale: a fresh sign-in is cheap and correct.
pub fn needs_refresh(token: &str) -> bool {
    let Some(expires_at) = expires_at(token) else {
        return true;
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);

    expires_at.saturating_sub(now) < REFRESH_MARGIN_SECS
}

fn expires_at(token: &str) -> Option<u64> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64url_decode(payload)?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;

    claims.get("exp")?.as_u64()
}

/// JWT payloads are base64url without padding, which is a small enough job to not be worth a
/// dependency.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let value = |byte: u8| -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some((byte - b'A') as u32),
            b'a'..=b'z' => Some((byte - b'a') as u32 + 26),
            b'0'..=b'9' => Some((byte - b'0') as u32 + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    };

    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }

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

    /// header.payload.signature, where the payload is `{"exp":<value>}` base64url encoded.
    fn token_expiring_at(exp: u64) -> String {
        let payload = format!(r#"{{"exp":{exp}}}"#);
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let bytes = payload.as_bytes();
        let mut encoded = String::new();

        for chunk in bytes.chunks(3) {
            let mut buffer = 0u32;

            for (index, byte) in chunk.iter().enumerate() {
                buffer |= (*byte as u32) << (16 - 8 * index);
            }

            for index in 0..chunk.len() + 1 {
                let shift = 18 - 6 * index;

                encoded.push(alphabet[((buffer >> shift) & 0x3f) as usize] as char);
            }
        }

        format!("header.{encoded}.signature")
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn a_fresh_token_is_kept() {
        assert!(!needs_refresh(&token_expiring_at(now() + 7 * 24 * 60 * 60)));
    }

    #[test]
    fn a_token_inside_the_margin_is_replaced() {
        assert!(needs_refresh(&token_expiring_at(now() + 60 * 60)));
    }

    #[test]
    fn an_expired_token_is_replaced() {
        assert!(needs_refresh(&token_expiring_at(now().saturating_sub(60))));
    }

    #[test]
    fn anything_unparseable_is_replaced() {
        assert!(needs_refresh(""));
        assert!(needs_refresh("not-a-jwt"));
        assert!(needs_refresh("header.!!!!.signature"));
    }
}
