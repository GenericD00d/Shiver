//! Shiver's read-only connection to a Sharkord server, spoken from the core rather than a page.
//!
//! Sequence (worked out against a live server; `examples/sharkord.rs probe` reproduces it):
//! 1. `POST /login` -> `{ token }` (done by the caller).
//! 2. Connect to `wss://host/?connectionParams=1`; without the query the server fails the upgrade.
//! 3. Send `{ method: "connectionParams", data: { token } }` to authenticate.
//! 4. Query `others.handshake`, then `others.joinServer` with its hash. Only after the join are
//!    protected procedures allowed; the join returns the initial state, including `readStates`.
//! 5. Query `dms.get` and (if the companion plugin is installed) `plugins.getUserData`.
//! 6. Subscribe to read-state deltas, read-state updates and new messages.
//!
//! It never sends anything on the user's behalf. Unread is accumulated the way Sharkord's own client
//! does: `readStates` from the join is the baseline, deltas add to it, updates set it.

use std::collections::{HashMap, HashSet};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use shiver_core::login::presentable;
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

pub use crate::check::{check_server, CheckedSessions, ServerCheck};
pub use crate::error::{Error, Result};

mod check;
mod error;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How long any one handshake step may take.
const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// How long a joined socket may be silent before it is treated as dead (tRPC pings every 30s).
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// Largest message accepted. The biggest real payload measured (a large server's `joinServer`) is
/// about 2.6 MB; this is tungstenite's own frame default.
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// The ceiling for a server the user has explicitly trusted: raised, never removed.
const MAX_FRAME_TRUSTED: usize = 128 * 1024 * 1024;

const HANDSHAKE_ID: u32 = 1;
const JOIN_ID: u32 = 2;
const DMS_ID: u32 = 3;
const PLUGIN_DATA_ID: u32 = 4;
const READ_STATE_ID: u32 = 10;
const MESSAGE_ID: u32 = 11;
const READ_STATE_UPDATE_ID: u32 = 12;

/// "One more unread here", published per arriving message.
const READ_STATE_PATH: &str = "channels.onReadStateDelta";
/// "This channel's count is now N", published when the user reads a channel on any device.
const READ_STATE_UPDATE_PATH: &str = "channels.onReadStateUpdate";
const MESSAGE_PATH: &str = "messages.onNew";

/// The id Shiver's companion plugin installs under.
const SHIVER_PLUGIN_ID: &str = "shiver";

/// One direct-message conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMessage {
    pub channel_id: i64,
    /// resolved here, because the page that draws it belongs to a different server
    pub user_name: String,
    /// the server's own `max(messages.createdAt)` for the conversation, in milliseconds
    pub last_message_at: Option<u64>,
}

/// What the server says about itself and this user when Shiver joins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Joined {
    pub server_id: Option<String>,
    pub own_user_id: Option<i64>,
    /// unread per channel id, the baseline later events are applied to
    pub read_states: HashMap<i64, u32>,
    pub dm_channels: Vec<i64>,
    pub channel_names: HashMap<i64, String>,
    /// every user's name, since any of them may post next
    pub user_names: HashMap<i64, String>,
    pub dms: Vec<DirectMessage>,
    /// the unread floor this user stored through the companion plugin, shared across devices
    pub shared_floor: Option<HashMap<i64, u32>>,
    /// the companion plugin's version, from `pluginsMetadata` (the only place a member can see it)
    pub plugin_version: Option<String>,
}

/// A message that arrived on a watched server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub channel_id: i64,
    /// absent for a message a plugin posted
    pub user_id: Option<i64>,
    pub plugin_id: Option<String>,
    /// the message as plain text
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// a channel's unread count moved by `delta` (a new message)
    Unread {
        channel_id: i64,
        delta: i64,
    },
    /// a channel's unread count is now `count` (it was read somewhere)
    UnreadSet {
        channel_id: i64,
        count: u32,
    },
    Posted(NewMessage),
}

/* ── the wire format ── */

/// The websocket address for an https origin. wss only: the first frame carries the session.
fn ws_url(origin: &str) -> Result<String> {
    origin
        .trim_end_matches('/')
        .strip_prefix("https://")
        .filter(|rest| !rest.is_empty() && !rest.contains(['/', '?', '#']))
        .map(|rest| format!("wss://{rest}/?connectionParams=1"))
        .ok_or_else(|| {
            Error::InvalidOrigin(format!(
                "'{origin}' is not an https origin, and Shiver will not carry a session over ws://"
            ))
        })
}

fn params_frame(token: &str) -> String {
    serde_json::json!({ "method": "connectionParams", "data": { "token": token } }).to_string()
}

/// One tRPC request. `input` is omitted entirely when `None` (void procedures refuse `null`).
fn request_frame(id: u32, method: &str, path: &str, input: Option<Value>) -> String {
    let mut params = serde_json::json!({ "path": path });

    if let Some(input) = input {
        params["input"] = input;
    }

    serde_json::json!({ "id": id, "jsonrpc": "2.0", "method": method, "params": params })
        .to_string()
}

/// A frame from the server, reduced to what Shiver acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reply {
    Data { id: Option<u64>, data: Value },
    Started { id: Option<u64> },
    Failed { id: Option<u64>, message: String },
    Other,
}

fn parse_reply(text: &str) -> Reply {
    let Ok(mut value) = serde_json::from_str::<Value>(text) else {
        return Reply::Other;
    };

    let id = value.get("id").and_then(Value::as_u64);

    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .and_then(presentable)
            .unwrap_or_else(|| "The server refused the request".into());

        return Reply::Failed { id, message };
    }

    let Some(result) = value.get_mut("result") else {
        return Reply::Other;
    };

    if result.get("type").and_then(Value::as_str) == Some("started") {
        return Reply::Started { id };
    }

    // taken rather than cloned: the join payload can be megabytes
    match result.get_mut("data") {
        Some(data) => Reply::Data {
            id,
            data: data.take(),
        },
        None => Reply::Other,
    }
}

/// A count from json, accepting doubles (a page only has doubles) and saturating at `u32::MAX`.
fn as_count(value: &Value) -> Option<u32> {
    if let Some(number) = value.as_u64() {
        return Some(number.min(u64::from(u32::MAX)) as u32);
    }

    let number = value.as_f64()?;

    (number.is_finite() && number >= 0.0).then(|| number.round().min(f64::from(u32::MAX)) as u32)
}

/// A signed integer from json, accepting a whole double.
fn as_integer(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|number| number.is_finite() && number.fract() == 0.0)
            .map(|number| number as i64)
    })
}

/// `{ "<channel id>": count }` maps, as `readStates` and the shared floor are stored.
fn channel_counts(value: &Value) -> Option<HashMap<i64, u32>> {
    Some(
        value
            .as_object()?
            .iter()
            .filter_map(|(channel, count)| Some((channel.parse().ok()?, as_count(count)?)))
            .collect(),
    )
}

/// The parts of `others.joinServer`'s answer Shiver uses. Missing fields read as empty.
fn parse_join(data: &Value) -> Joined {
    let mut joined = Joined {
        server_id: data
            .get("serverId")
            .and_then(Value::as_str)
            .map(str::to_string),
        own_user_id: data.get("ownUserId").and_then(Value::as_i64),
        read_states: data
            .get("readStates")
            .and_then(channel_counts)
            .unwrap_or_default(),
        plugin_version: plugin_version(data, SHIVER_PLUGIN_ID),
        ..Joined::default()
    };

    for channel in data
        .get("channels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = channel.get("id").and_then(Value::as_i64) else {
            continue;
        };

        if channel.get("isDm").and_then(Value::as_bool) == Some(true) {
            joined.dm_channels.push(id);
        }

        if let Some(name) = channel.get("name").and_then(Value::as_str) {
            joined.channel_names.insert(id, name.to_string());
        }
    }

    joined.user_names = data
        .get("users")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|user| {
            Some((
                user.get("id").and_then(Value::as_i64)?,
                user.get("name").and_then(Value::as_str)?.to_string(),
            ))
        })
        .collect();

    joined
}

fn plugin_version(data: &Value, plugin_id: &str) -> Option<String> {
    data.get("pluginsMetadata")?
        .as_array()?
        .iter()
        .find(|plugin| plugin.get("pluginId").and_then(Value::as_str) == Some(plugin_id))?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

/// `dms.get`'s answer, named from the join's user list. Unknown users read as "Unknown".
fn parse_dms(conversations: &[Value], user_names: &HashMap<i64, String>) -> Vec<DirectMessage> {
    conversations
        .iter()
        .filter_map(|dm| {
            let user_id = dm.get("userId").and_then(Value::as_i64)?;

            Some(DirectMessage {
                channel_id: dm.get("channelId").and_then(Value::as_i64)?,
                user_name: user_names
                    .get(&user_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown".into()),
                last_message_at: dm.get("lastMessageAt").and_then(|at| {
                    at.as_u64().or_else(|| {
                        at.as_f64()
                            .filter(|at| *at >= 0.0)
                            .map(|at| at.round() as u64)
                    })
                }),
            })
        })
        .collect()
}

/// The shared unread floor from the plugin's row, if it holds one.
fn parse_shared_floor(stored: &Value) -> Option<HashMap<i64, u32>> {
    stored.get("readFloor").and_then(channel_counts)
}

fn parse_message(data: &Value) -> Option<Event> {
    Some(Event::Posted(NewMessage {
        channel_id: data.get("channelId").and_then(Value::as_i64)?,
        user_id: data.get("userId").and_then(Value::as_i64),
        plugin_id: data
            .get("pluginId")
            .and_then(Value::as_str)
            .map(str::to_string),
        text: plain_text(data.get("content").and_then(Value::as_str).unwrap_or("")),
    }))
}

/// A read-state frame: `delta` for a new message, `count` for a read.
fn parse_delta(data: &Value) -> Option<Event> {
    let channel_id = data.get("channelId").and_then(Value::as_i64)?;

    if let Some(delta) = data.get("delta").and_then(as_integer) {
        return Some(Event::Unread { channel_id, delta });
    }

    Some(Event::UnreadSet {
        channel_id,
        count: data.get("count").and_then(as_count)?,
    })
}

/// Sharkord's message html as one line of text for a notification. Tags become word breaks, common
/// entities are decoded (named and numeric) and whitespace collapses. Not a sanitiser: the result
/// is never put back into html.
fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut pending_space = false;
    let mut rest = html;

    let push = |text: &mut String, pending_space: &mut bool, piece: &str| {
        if *pending_space && !text.is_empty() {
            text.push(' ');
        }

        *pending_space = false;
        text.push_str(piece);
    };

    while let Some(character) = rest.chars().next() {
        let after = &rest[character.len_utf8()..];

        match character {
            // only a real tag: `<` followed by a letter, `/` or `!`, and closed by a `>`
            '<' if after.starts_with(|next: char| {
                next.is_ascii_alphabetic() || next == '/' || next == '!'
            }) && after.contains('>') =>
            {
                rest = &after[after.find('>').unwrap_or(0) + 1..];
                pending_space = true;

                continue;
            }
            '&' => {
                if let Some((decoded, length)) = entity(after) {
                    match decoded {
                        ' ' => pending_space = true,
                        other => push(
                            &mut text,
                            &mut pending_space,
                            other.encode_utf8(&mut [0; 4]),
                        ),
                    }

                    rest = &after[length..];

                    continue;
                }

                push(&mut text, &mut pending_space, "&");
            }
            _ if character.is_whitespace() => pending_space = true,
            _ => push(
                &mut text,
                &mut pending_space,
                character.encode_utf8(&mut [0; 4]),
            ),
        }

        rest = after;
    }

    text
}

/// Decodes an entity at the start of `text` (after its `&`): the character and how many bytes it
/// used, including the `;`.
fn entity(text: &str) -> Option<(char, usize)> {
    let end = text
        .char_indices()
        .take(12)
        .find(|(_, character)| *character == ';')?
        .0;
    let name = &text[..end];

    let decoded = match name {
        "nbsp" => ' ',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "amp" => '&',
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };

            char::from_u32(code).filter(|character| !character.is_control())?
        }
    };

    Some((decoded, end + 1))
}

/// Sets one channel's unread count outright (a read reported by another device).
pub fn set_unread(read_states: &mut HashMap<i64, u32>, channel_id: i64, count: u32) {
    if count == 0 {
        read_states.remove(&channel_id);
    } else {
        read_states.insert(channel_id, count);
    }
}

/// Unread above the baseline, per channel, skipping muted channels. A channel below its baseline
/// (read elsewhere) contributes nothing rather than hiding news in another channel.
pub fn unread_total(
    read_states: &HashMap<i64, u32>,
    baseline: &HashMap<i64, u32>,
    muted: &HashSet<i64>,
) -> u32 {
    read_states
        .iter()
        .filter(|(channel, _)| !muted.contains(*channel))
        .map(|(channel, count)| count.saturating_sub(baseline.get(channel).copied().unwrap_or(0)))
        .fold(0u32, u32::saturating_add)
}

/// Applies a delta to a channel's count, clamped to `0..=u32::MAX`.
pub fn apply_delta(read_states: &mut HashMap<i64, u32>, channel_id: i64, delta: i64) {
    let entry = read_states.entry(channel_id).or_insert(0);

    *entry = i64::from(*entry)
        .saturating_add(delta)
        .clamp(0, i64::from(u32::MAX)) as u32;
}

/* ── the connection ── */

/// How long to wait before reconnecting after `attempt` consecutive failures: 30s doubling to a
/// 10-minute ceiling, plus a stable per-server offset (from `key`) so a whole rail does not
/// reconnect in lockstep.
fn retry_delay(attempt: u32, key: &str) -> std::time::Duration {
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
    pub token: String,
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
    /// A message exceeded the frame limit (retrying will not fix it).
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
                failures = 0;
                refusals = 0;
                watcher.joined(&session.joined);

                while let Some(event) = session.next_event().await {
                    watcher.event(&session.joined, event);
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
            }
            Err(error) => eprintln!("[shiver] could not watch {origin}: {error}"),
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

    send(&mut socket, params_frame(token)).await?;

    let handshake = call(
        &mut socket,
        HANDSHAKE_ID,
        "others.handshake",
        Some(Value::Null),
    )
    .await?;
    let hash = handshake
        .get("handshakeHash")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::NotSharkord(origin.to_string()))?
        .to_string();

    let mut joined = parse_join(
        &call(
            &mut socket,
            JOIN_ID,
            "others.joinServer",
            Some(serde_json::json!({ "handshakeHash": hash })),
        )
        .await?,
    );

    // Optional extras: failing either costs that feature, not the connection.
    match call(&mut socket, DMS_ID, "dms.get", None).await {
        Ok(Value::Array(conversations)) => {
            joined.dms = parse_dms(&conversations, &joined.user_names)
        }
        Ok(_) => eprintln!("[shiver] {origin}: dms.get did not answer with a list"),
        Err(error) => eprintln!("[shiver] could not read {origin}'s direct messages: {error}"),
    }

    if joined.plugin_version.is_some() {
        let input = serde_json::json!({ "pluginId": SHIVER_PLUGIN_ID });

        match call(
            &mut socket,
            PLUGIN_DATA_ID,
            "plugins.getUserData",
            Some(input),
        )
        .await
        {
            Ok(stored) => joined.shared_floor = parse_shared_floor(&stored),
            Err(error) => {
                eprintln!("[shiver] could not read {origin}'s shared unread floor: {error}")
            }
        }
    }

    for (id, path) in [
        (READ_STATE_ID, READ_STATE_PATH),
        (READ_STATE_UPDATE_ID, READ_STATE_UPDATE_PATH),
        (MESSAGE_ID, MESSAGE_PATH),
    ] {
        send(
            &mut socket,
            request_frame(id, "subscription", path, Some(Value::Null)),
        )
        .await?;
    }

    Ok(Session { socket, joined })
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
                Reply::Failed { id, message } => {
                    eprintln!("[shiver] the server refused request {id:?}: {message}");

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

        match parse_reply(&text) {
            Reply::Data {
                id: Some(reply),
                data,
            } if reply == u64::from(id) => return Ok(data),
            Reply::Failed {
                id: Some(reply),
                message,
            } if reply == u64::from(id) => return Err(Error::Refused(message)),
            _ => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn muted<const N: usize>(channels: [i64; N]) -> HashSet<i64> {
        channels.into_iter().collect()
    }

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

    #[test]
    fn websocket_urls_are_wss_with_the_params_query_and_nothing_else() {
        assert_eq!(
            ws_url("https://chat.example.com").unwrap(),
            "wss://chat.example.com/?connectionParams=1"
        );
        assert_eq!(
            ws_url("https://localhost:4991/").unwrap(),
            "wss://localhost:4991/?connectionParams=1"
        );

        for bad in [
            "http://chat.example.com",
            "ftp://example.com",
            "https://x/path",
            "https://",
        ] {
            assert!(ws_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn plain_text_strips_tags_and_keeps_less_than_signs() {
        assert_eq!(plain_text("1 < 2 and 3 > 4"), "1 < 2 and 3 > 4");
        assert_eq!(plain_text("5<6"), "5<6");
        // `<` + letter with no closing `>` is text, not the start of a tag that eats the rest
        assert_eq!(plain_text("i think a <b and c"), "i think a <b and c");
        assert_eq!(plain_text("<p>hello</p><p>there</p>"), "hello there");
        assert_eq!(plain_text("<img src=\"x\">caption"), "caption");
        assert_eq!(plain_text("<!-- note -->text"), "text");
        assert_eq!(plain_text("  spaced   out  "), "spaced out");
        assert_eq!(plain_text("<p></p>"), "");
    }

    #[test]
    fn entities_are_decoded_once() {
        assert_eq!(plain_text("&lt;tag&gt;"), "<tag>");
        assert_eq!(plain_text("a&nbsp;b"), "a b");
        assert_eq!(
            plain_text("it&#39;s &#x2019; &quot;q&quot;"),
            "it's \u{2019} \"q\""
        );
        assert_eq!(plain_text("&amp;lt;"), "&lt;");
        assert_eq!(plain_text("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(plain_text("&unknown;"), "&unknown;");
        assert_eq!(plain_text("&#0; &#x7;"), "&#0; &#x7;");
    }

    #[test]
    fn frames_are_built_as_the_server_expects() {
        let params: Value = serde_json::from_str(&params_frame("abc")).unwrap();

        assert_eq!(params["method"], "connectionParams");
        assert_eq!(params["data"]["token"], "abc");

        let query: Value = serde_json::from_str(&request_frame(
            7,
            "query",
            "others.handshake",
            Some(Value::Null),
        ))
        .unwrap();

        assert_eq!(
            (query["id"].as_u64(), query["params"]["path"].as_str()),
            (Some(7), Some("others.handshake"))
        );
        assert!(query["params"].get("input").is_some());

        let void: Value =
            serde_json::from_str(&request_frame(3, "query", "dms.get", None)).unwrap();

        assert!(
            void["params"].get("input").is_none(),
            "void procedures refuse an input key"
        );
    }

    #[test]
    fn replies_are_classified() {
        match parse_reply(r#"{"id":1,"result":{"type":"data","data":{"handshakeHash":"h"}}}"#) {
            Reply::Data { id, data } => {
                assert_eq!((id, data["handshakeHash"].as_str()), (Some(1), Some("h")))
            }
            other => panic!("expected data, got {other:?}"),
        }

        assert_eq!(
            parse_reply(r#"{"id":10,"result":{"type":"started"}}"#),
            Reply::Started { id: Some(10) }
        );
        assert_eq!(
            parse_reply(
                r#"{"id":10,"error":{"message":"You must be authenticated.","code":-32001}}"#
            ),
            Reply::Failed {
                id: Some(10),
                message: "You must be authenticated.".into()
            }
        );
        assert_eq!(
            parse_reply(&format!(
                r#"{{"id":2,"error":{{"message":"see https://evil.example {}"}}}}"#,
                "x".repeat(300)
            )),
            Reply::Failed {
                id: Some(2),
                message: format!("see {}", "x".repeat(196))
            }
        );
        assert_eq!(parse_reply("not json"), Reply::Other);
    }

    #[test]
    fn the_join_payload_yields_what_shiver_uses() {
        let joined = parse_join(&serde_json::json!({
            "serverId": "019c1482",
            "readStates": { "1": 0, "8": 3, "140": 2.0 },
            "channels": [{ "id": 1, "isDm": false, "name": "README" }, { "id": 9, "isDm": true, "name": "x" }],
            "users": [{ "id": 2, "name": "Smiddy" }],
            "pluginsMetadata": [{ "pluginId": "other", "version": "2.0.0" }, { "pluginId": "shiver", "version": "0.1.0" }]
        }));

        assert_eq!(joined.server_id.as_deref(), Some("019c1482"));
        assert_eq!(joined.read_states.get(&140), Some(&2));
        assert_eq!(joined.dm_channels, vec![9]);
        assert_eq!(
            joined.user_names.get(&2).map(String::as_str),
            Some("Smiddy")
        );
        assert_eq!(joined.plugin_version.as_deref(), Some("0.1.0"));
        assert_eq!(parse_join(&serde_json::json!({})), Joined::default());
    }

    #[test]
    fn conversations_carry_their_name_and_latest_message() {
        let names = HashMap::from([(2, "Smiddy".to_string())]);
        let dms = parse_dms(
            serde_json::json!([
                { "channelId": 9, "userId": 2, "lastMessageAt": 1_757_000_000_000u64 },
                { "channelId": 10, "userId": 404, "lastMessageAt": 1.757e12 },
                { "channelId": 11 }
            ])
            .as_array()
            .unwrap(),
            &names,
        );

        assert_eq!(dms.len(), 2);
        assert_eq!(
            (dms[0].user_name.as_str(), dms[0].last_message_at),
            ("Smiddy", Some(1_757_000_000_000))
        );
        assert_eq!(
            (dms[1].user_name.as_str(), dms[1].last_message_at),
            ("Unknown", Some(1_757_000_000_000))
        );
    }

    #[test]
    fn read_state_frames_accept_doubles() {
        assert_eq!(
            parse_delta(&serde_json::json!({ "channelId": 8, "delta": 1 })),
            Some(Event::Unread {
                channel_id: 8,
                delta: 1
            })
        );
        assert_eq!(
            parse_delta(&serde_json::json!({ "channelId": 8, "delta": 1.0 })),
            Some(Event::Unread {
                channel_id: 8,
                delta: 1
            })
        );
        assert_eq!(
            parse_delta(&serde_json::json!({ "channelId": 4, "count": 0.0 })),
            Some(Event::UnreadSet {
                channel_id: 4,
                count: 0
            })
        );
        assert_eq!(parse_delta(&serde_json::json!({ "channelId": 8 })), None);
        assert_eq!(parse_delta(&serde_json::json!({ "delta": 1 })), None);
    }

    #[test]
    fn new_messages_are_read_from_the_frame_the_server_sends() {
        let data = serde_json::json!({
            "id": 13723, "content": "<p>shape check</p>", "userId": 52737, "pluginId": null, "channelId": 8
        });

        assert_eq!(
            parse_message(&data),
            Some(Event::Posted(NewMessage {
                channel_id: 8,
                user_id: Some(52737),
                plugin_id: None,
                text: "shape check".into()
            }))
        );
        assert_eq!(parse_message(&serde_json::json!({ "content": "hi" })), None);
    }

    #[test]
    fn the_shared_floor_is_read_tolerantly() {
        let floor = parse_shared_floor(&serde_json::json!({
            "readFloor": { "12": 3, "4": 12.0, "5": 4_294_967_296u64, "no": 1, "40": "lots" }
        }))
        .unwrap();

        assert_eq!(floor, HashMap::from([(12, 3), (4, 12), (5, u32::MAX)]));
        assert!(parse_shared_floor(&serde_json::json!({ "mutedChannels": [1] })).is_none());
        assert!(parse_shared_floor(&serde_json::json!({ "readFloor": 7 })).is_none());
    }

    #[test]
    fn unread_counts_only_what_arrived_above_the_baseline() {
        let states = HashMap::from([(1, 42), (2, 900), (3, 4)]);
        let baseline = HashMap::from([(1, 40), (2, 900)]);

        assert_eq!(unread_total(&states, &baseline, &muted([])), 6);
        assert_eq!(unread_total(&states, &baseline, &muted([3])), 2);
        assert_eq!(
            unread_total(
                &HashMap::from([(1, 0)]),
                &HashMap::from([(1, 10)]),
                &muted([])
            ),
            0
        );
    }

    #[test]
    fn deltas_accumulate_without_going_negative_and_sets_replace() {
        let mut states = HashMap::from([(1, 1)]);

        apply_delta(&mut states, 1, 1);
        apply_delta(&mut states, 2, 1);
        assert_eq!((states[&1], states[&2]), (2, 1));

        apply_delta(&mut states, 1, -5);
        assert_eq!(states[&1], 0);

        set_unread(&mut states, 2, 7);
        assert_eq!(states[&2], 7);

        set_unread(&mut states, 2, 0);
        assert!(!states.contains_key(&2));
    }
}
