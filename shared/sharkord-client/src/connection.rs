//! The connection: open, authenticate, join and subscribe, then read events until it ends; and
//! `watch`, which keeps one open with backoff.

use std::collections::HashSet;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::{
    wire::{
        params_frame, parse_delta, parse_dms, parse_join, parse_message, parse_reply,
        parse_shared_floor, request_frame, ws_url, Reply, DMS_ID, HANDSHAKE_ID, JOIN_ID,
        MESSAGE_ID, PLUGIN_DATA_ID, READ_STATE_ID, READ_STATE_UPDATE_ID, SHIVER_PLUGIN_ID,
        SUBSCRIPTIONS,
    },
    Error, Event, Joined, Result,
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How long any one handshake step may take.
const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// How long a joined socket may be silent before it is treated as dead (tRPC pings every 30s).
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// How long a connection must have lasted, if it delivered nothing, to count as having worked.
const STABLE_CONNECTION: std::time::Duration = std::time::Duration::from_secs(60);

/// Largest message accepted. The biggest real payload measured (a large server's `joinServer`) is
/// about 2.6 MB; this is tungstenite's own frame default.
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// The ceiling for a server the user has explicitly trusted: raised, never removed.
const MAX_FRAME_TRUSTED: usize = 128 * 1024 * 1024;

/// How long to wait before reconnecting after `attempt` consecutive failures: 30s doubling to a
/// 10-minute ceiling, plus a stable per-server offset (from `key`) so a whole rail does not
/// reconnect in lockstep.
pub fn retry_delay(attempt: u32, key: &str) -> std::time::Duration {
    const FIRST: std::time::Duration = std::time::Duration::from_secs(30);
    const CEILING: std::time::Duration = std::time::Duration::from_secs(10 * 60);

    let backed_off = FIRST.saturating_mul(1u32 << attempt.min(5)).min(CEILING);
    let spread = key.bytes().fold(0u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u64::from(byte))
    });
    let jitter = backed_off.as_millis() as u64 / 4;

    backed_off + std::time::Duration::from_millis(spread.checked_rem(jitter).unwrap_or(0))
}

/// Where and how to connect for one attempt.
pub struct Target {
    pub origin: String,
    /// wiped when the attempt is over
    pub token: zeroize::Zeroizing<String>,
    pub accept_any_size: bool,
}

/// The client-specific half of [`watch`].
pub trait Watcher: Send {
    /// What to connect with next; `None` stops watching.
    fn target(&mut self) -> impl std::future::Future<Output = Option<Target>> + Send;
    fn joined(&mut self, joined: &Joined);
    fn event(&mut self, joined: &Joined, event: Event);
    /// The server refused the session (`refusals` in a row). `true` retries at once (after the
    /// watcher renewed the session); `false` stops watching.
    fn refused(&mut self, refusals: u32) -> impl std::future::Future<Output = bool> + Send;
    /// A message exceeded the frame limit. Watching stops, since retrying will not fix it.
    fn too_large(&mut self, size: usize);
    /// The connection is down, before the backoff.
    fn disconnected(&mut self) {}
}

/// Holds one server's connection open, reconnecting with `retry_delay` backoff (keyed by `key`)
/// until the watcher stops it.
pub async fn watch(key: &str, mut watcher: impl Watcher) {
    let mut failures: u32 = 0;
    let mut refusals: u32 = 0;

    while let Some(target) = watcher.target().await {
        let origin = &target.origin;

        match open(origin, &target.token, target.accept_any_size).await {
            Ok(mut session) => {
                let opened = std::time::Instant::now();
                let mut delivered = false;

                refusals = 0;
                watcher.joined(&session.joined);

                while let Some(event) = session.next_event().await {
                    delivered = true;
                    watcher.event(&session.joined, event);
                }

                // a connection that worked starts the backoff over; one closed straight after
                // joining (a refused subscription) keeps backing off
                if delivered || opened.elapsed() >= STABLE_CONNECTION {
                    failures = 0;
                }

                eprintln!("[shiver] {origin} closed the connection");
            }
            Err(Error::Refused(reason)) => {
                eprintln!("[shiver] {origin} refused Shiver's session ({reason})");
                refusals += 1;

                if watcher.refused(refusals).await {
                    continue;
                }

                return;
            }
            Err(Error::TooLarge { size, max }) => {
                eprintln!(
                    "[shiver] {origin} sent {size} bytes in one message; Shiver accepts {max}"
                );
                watcher.too_large(size);

                return;
            }
            Err(error) => eprintln!("[shiver] could not watch a server: {error}"),
        }

        watcher.disconnected();
        tokio::time::sleep(retry_delay(failures, key)).await;
        failures = failures.saturating_add(1);
    }
}

/// `TooLarge` for a size limit (it will not fix itself), `Unreachable` for anything else.
fn socket_error(error: tokio_tungstenite::tungstenite::Error) -> Error {
    use tokio_tungstenite::tungstenite::{error::CapacityError, Error as Tungstenite};

    match error {
        Tungstenite::Capacity(CapacityError::MessageTooLong { size, max_size }) => {
            Error::TooLarge {
                size,
                max: max_size,
            }
        }
        other => Error::Unreachable(other.to_string()),
    }
}

fn frame_cap(accept_any_size: bool) -> usize {
    if accept_any_size {
        MAX_FRAME_TRUSTED
    } else {
        MAX_FRAME
    }
}

/// A live, joined connection to one server.
pub(crate) struct Session {
    socket: Socket,
    pub(crate) joined: Joined,
    /// subscriptions the server refused; once it has refused them all, nothing more can arrive
    refused: HashSet<u64>,
}

/// Connects, authenticates, joins and subscribes. Returns once the join has been answered.
pub(crate) async fn open(origin: &str, token: &str, accept_any_size: bool) -> Result<Session> {
    let url = ws_url(origin)?;
    let cap = frame_cap(accept_any_size);
    let limits = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(cap),
        max_frame_size: Some(cap),
        ..Default::default()
    };

    let (mut socket, _) = tokio::time::timeout(
        STEP_TIMEOUT,
        tokio_tungstenite::connect_async_with_config(&url, Some(limits), false),
    )
    .await
    .map_err(|_| Error::Unreachable(format!("{origin} (timed out)")))?
    .map_err(|error| Error::Unreachable(format!("{origin} ({error})")))?;

    // every failure names the server, whichever step it came from
    let joined = join(&mut socket, origin, token)
        .await
        .map_err(|error| match error {
            Error::Unreachable(detail) => Error::Unreachable(format!("{origin} ({detail})")),
            other => other,
        })?;

    Ok(Session {
        socket,
        joined,
        refused: HashSet::new(),
    })
}

/// Authenticates, joins and subscribes on a connected socket.
pub(crate) async fn join(socket: &mut Socket, origin: &str, token: &str) -> Result<Joined> {
    send(socket, params_frame(token)).await?;

    let handshake = call(socket, HANDSHAKE_ID, "others.handshake", Some(Value::Null)).await?;
    let hash = handshake
        .get("handshakeHash")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::NotSharkord(origin.to_string()))?
        .to_string();

    let mut joined = parse_join(
        &call(
            socket,
            JOIN_ID,
            "others.joinServer",
            Some(serde_json::json!({ "handshakeHash": hash })),
        )
        .await?,
    );

    // Optional extras: failing either costs that feature, not the connection.
    match call(socket, DMS_ID, "dms.get", None).await {
        Ok(Value::Array(conversations)) => joined.dms = parse_dms(&conversations, &joined, origin),
        Ok(_) => eprintln!("[shiver] {origin}: dms.get did not answer with a list"),
        Err(error) => eprintln!("[shiver] could not read {origin}'s direct messages: {error}"),
    }

    if joined.plugin_version.is_some() {
        let input = serde_json::json!({ "pluginId": SHIVER_PLUGIN_ID });

        match call(socket, PLUGIN_DATA_ID, "plugins.getUserData", Some(input)).await {
            Ok(stored) => joined.shared_floor = parse_shared_floor(&stored),
            Err(error) => {
                eprintln!("[shiver] could not read {origin}'s shared unread floor: {error}")
            }
        }
    }

    for (id, path) in SUBSCRIPTIONS {
        send(
            socket,
            request_frame(id, "subscription", path, Some(Value::Null)),
        )
        .await?;
    }

    Ok(joined)
}

impl Session {
    /// The next event, or `None` once the connection has ended or gone silent for `IDLE_TIMEOUT`.
    /// Answers tRPC's `PING` (the server drops sockets that do not) and skips unknown frames.
    async fn next_event(&mut self) -> Option<Event> {
        loop {
            let frame = match tokio::time::timeout(IDLE_TIMEOUT, self.socket.next()).await {
                Ok(Some(Ok(frame))) => frame,
                Ok(Some(Err(error))) => {
                    eprintln!("[shiver] a server's socket failed mid-session: {error}");

                    return None;
                }
                Ok(None) => return None,
                Err(_) => {
                    eprintln!(
                        "[shiver] a server said nothing for {}s; dropping the socket",
                        IDLE_TIMEOUT.as_secs()
                    );

                    return None;
                }
            };

            let text = match frame {
                Message::Text(text) => text,
                Message::Close(_) => return None,
                _ => continue,
            };

            if text == "PING" {
                if self
                    .socket
                    .send(Message::Text("PONG".into()))
                    .await
                    .is_err()
                {
                    return None;
                }

                continue;
            }

            let event = match parse_reply(&text) {
                Reply::Data { id: Some(id), data }
                    if id == u64::from(READ_STATE_ID) || id == u64::from(READ_STATE_UPDATE_ID) =>
                {
                    parse_delta(&data)
                }
                Reply::Data { id: Some(id), data } if id == u64::from(MESSAGE_ID) => {
                    parse_message(&data)
                }
                Reply::Failed { id, message, .. } => {
                    eprintln!("[shiver] the server refused request {id:?}: {message}");

                    // with every subscription refused the socket answers pings and says nothing:
                    // closed, so the watch reconnects (with backoff) rather than going quiet. One
                    // refused alone still leaves the others delivering.
                    if let Some(id) = id.filter(|id| {
                        SUBSCRIPTIONS
                            .iter()
                            .any(|(subscription, _)| *id == u64::from(*subscription))
                    }) {
                        self.refused.insert(id);

                        if self.refused.len() == SUBSCRIPTIONS.len() {
                            return None;
                        }
                    }

                    None
                }
                _ => None,
            };

            if event.is_some() {
                return event;
            }
        }
    }
}

async fn send(socket: &mut Socket, frame: String) -> Result<()> {
    socket
        .send(Message::Text(frame))
        .await
        .map_err(|error| Error::Unreachable(error.to_string()))
}

/// Sends one query and waits (up to `STEP_TIMEOUT`) for the reply with the same id.
async fn call(socket: &mut Socket, id: u32, path: &str, input: Option<Value>) -> Result<Value> {
    send(socket, request_frame(id, "query", path, input)).await?;

    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;

    loop {
        let frame = tokio::time::timeout_at(deadline, socket.next())
            .await
            .map_err(|_| Error::Unreachable(format!("{path} did not answer")))?
            .ok_or_else(|| Error::Unreachable(format!("{path}: the connection closed")))?
            .map_err(socket_error)?;

        let Message::Text(text) = frame else {
            continue;
        };

        // the server pings during a slow step too, and drops a socket that does not answer
        if text == "PING" {
            send(socket, "PONG".into()).await?;

            continue;
        }

        match parse_reply(&text) {
            Reply::Data {
                id: Some(reply),
                data,
            } if reply == u64::from(id) => return Ok(data),
            Reply::Failed {
                id: Some(reply),
                message,
                busy,
            } if reply == u64::from(id) => {
                return Err(if busy {
                    Error::Busy(message)
                } else {
                    Error::Refused(message)
                })
            }
            // an error naming no request (one the server could not read) ends the step now rather
            // than at its timeout; retried like a dropped connection, since it says nothing about
            // the session
            Reply::Failed {
                id: None, message, ..
            } => return Err(Error::Unreachable(format!("{path}: {message}"))),
            _ => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_to_a_ceiling_with_a_stable_offset() {
        let first = std::time::Duration::from_secs(30);
        let ceiling = std::time::Duration::from_secs(600);

        assert!(retry_delay(0, "a") >= first);
        assert!(retry_delay(1, "a") > retry_delay(0, "a"));
        assert!(retry_delay(30, "a") <= ceiling + ceiling / 4);
        assert_eq!(retry_delay(2, "a"), retry_delay(2, "a"));
    }

    #[test]
    fn trusting_a_server_raises_the_ceiling_but_keeps_one() {
        assert!(frame_cap(true) > frame_cap(false));
        assert!(frame_cap(true) <= 256 * 1024 * 1024);
    }

    #[test]
    fn only_a_capacity_failure_reads_as_too_large() {
        use tokio_tungstenite::tungstenite::{error::CapacityError, Error as Tungstenite};

        let too_big = socket_error(Tungstenite::Capacity(CapacityError::MessageTooLong {
            size: 40_000_000,
            max_size: MAX_FRAME,
        }));

        assert!(matches!(
            too_big,
            Error::TooLarge {
                size: 40_000_000,
                max: MAX_FRAME
            }
        ));
        assert!(matches!(
            socket_error(Tungstenite::ConnectionClosed),
            Error::Unreachable(_)
        ));
    }
}
