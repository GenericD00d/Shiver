//! The http calls Shiver makes for itself, and the two rules they all obey.
//!
//! **No redirect is ever followed.** A server answering with a `Location` is a server choosing
//! where Shiver's own process connects next — to a host on the user's network, or to a third party
//! that learns the device's address from being contacted. On `/login` it is worse than that: a 307
//! keeps the method *and the body*, so the password goes with it.
//!
//! **No body is unbounded.** The websocket path reasons carefully about frame caps because Shiver
//! holds one socket per server; the http calls to those same servers used to read whatever arrived
//! straight into memory. `/info` is reachable from the add dialog before a server is even in the
//! rail.

use std::time::Duration;

use serde_json::Value;

/// The most Shiver will read from one http response.
///
/// These answers are a handful of fields — a server name, a logo row, a session token. A megabyte
/// is orders of magnitude more than any of them and still small enough that a hostile answer costs
/// nothing worth noticing.
pub const MAX_BODY: usize = 1024 * 1024;

/// Builds the client every call here uses.
pub fn client(timeout: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Shiver/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Reads a response as json, giving up once it has read more than [`MAX_BODY`].
///
/// Streamed rather than `response.json()`, because that buffers the whole body first — so the
/// limit has to be applied while it arrives rather than after.
pub async fn json_within_limit(response: reqwest::Response) -> Option<Value> {
    use futures_util::StreamExt;

    // a declared length over the cap is refused before a byte of it is read
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return None;
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;

        if body.len() + chunk.len() > MAX_BODY {
            return None;
        }

        body.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&body).ok()
}
