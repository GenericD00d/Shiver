//! Shiver's own http calls: one pooled client that never follows redirects, and bounded body reads.
//!
//! No redirects, because a `307` on `/login` would resend the password to wherever the server
//! names. No unbounded bodies, because every server Shiver talks to is untrusted.

use std::{sync::OnceLock, time::Duration};

use futures_util::StreamExt;
use serde_json::Value;

/// The most Shiver reads from one json answer.
pub const MAX_BODY: usize = 1024 * 1024;

/// How long any one request may take. Callers may shorten it per request.
const TIMEOUT: Duration = Duration::from_secs(20);

/// The shared client, built once so connections and TLS configuration are reused.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("Shiver/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("a client with no custom TLS configuration always builds")
    })
}

/// The body, or `None` once it exceeds `limit` bytes (declared or actual) or the stream fails.
pub async fn bytes_within_limit(response: reqwest::Response, limit: usize) -> Option<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return None;
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;

        if body.len() + chunk.len() > limit {
            return None;
        }

        body.extend_from_slice(&chunk);
    }

    Some(body)
}

/// The body as json, read within [`MAX_BODY`].
pub async fn json_within_limit(response: reqwest::Response) -> Option<Value> {
    serde_json::from_slice(&bytes_within_limit(response, MAX_BODY).await?).ok()
}
