//! Shiver's own connection to a Sharkord server, spoken from the core rather than from a page.
//!
//! Android gives a window one webview, so a server the user is not looking at has no page to report
//! from — which is why the mobile client had no unified inbox. This is how Shiver hears from it
//! anyway: it speaks the server's own tRPC-over-WebSocket protocol directly. It is **read-only**.
//! It joins, reads unread counts, and listens; it never sends anything on the user's behalf.
//!
//! The sequence was established against a live server rather than from tRPC's documentation, each
//! step found by being refused at the one before it (`examples/wsprobe.rs` is the probe that found
//! it, kept because the next person to touch this will want it):
//!
//! 1. `POST /login` → `{ token }`. Same endpoint desktop's `login.rs` uses.
//! 2. Connect to `wss://host/?connectionParams=1`. **The query matters**: without it the server
//!    builds its connection context immediately, finds `info.connectionParams` null and throws
//!    while the socket is still upgrading. tRPC only waits for a params message when asked to.
//! 3. Send `{ method: "connectionParams", data: { token } }`. This authenticates the socket.
//! 4. Query `others.handshake` → `{ handshakeHash }`.
//! 5. Query `others.joinServer` with that hash. This is what sets `ctx.authenticated`; a token
//!    alone is not enough, and every `protectedProcedure` refuses until it has run. It answers
//!    with the whole initial state, including `readStates`.
//! 6. Subscribe. `channels.onReadStateDelta` is the one Shiver needs.
//!
//! **Both clients use this crate.** It used to be a module inside `mobile/`, because Android was
//! the only place that needed it — a phone gives a window one webview, so a server not on screen
//! had no page to report from. Desktop now wants the same thing for a different reason: a webview
//! per server costs far more memory than a socket does, and people are in dozens of servers. What
//! is transcribed here was worked out by probing a live server, and a second copy of it would start
//! drifting the day one client learned something the other did not.
//!
//! //! Unread is modelled the way Sharkord's own client models it: the `readStates` map from the join
//! is the baseline, and each delta event adds to one channel's count. The server deliberately sends
//! a delta of 1 rather than a recomputed total — see `db/publishers.ts` — so Shiver must accumulate.

use std::collections::HashMap;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

pub use crate::error::{Error, Result};

mod error;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How long any single step of the handshake may take before Shiver gives up on the server.
const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The most one frame from a server may be.
///
/// `connect_async` defaults to 64 MiB per message and 16 MiB per frame, and Shiver holds one of these
/// sockets *per server* — so a handful of hostile servers could ask Shiver's phone process for half a
/// gigabyte between them and be within the library's limits. A bound is still wanted; the number is
/// the part that was guessed.
///
/// **It was 256 KiB, on the reasoning that the channel and user list arriving with `joinServer` is
/// the biggest legitimate frame and could not be large.** Measured on the emulator against a real
/// server, that frame was **2.6 MB** — so Shiver refused it, dropped the socket, and quietly stopped
/// watching that server altogether: no unread badge, no notifications, and nothing said about it
/// beyond one debug line. A guess about somebody else's payload turned into a feature that silently
/// did not work on any server big enough.
///
/// **16 MiB**, which is tungstenite's own frame default and about six times the only real payload
/// anyone has measured. The number moved up from 4 MiB once this failure started announcing itself:
/// a limit that is too small is now two taps from being fixed by the person it affects, while a
/// limit that is too large is a phone killed for memory, which is silent and takes the
/// notifications with it. Given that asymmetry, the default is set where a legitimate server is
/// very unlikely to meet it.
///
/// The exposure is transient and per socket — Shiver opens one per server, all of them at launch — so
/// the worst case is this times the number of servers, and the realistic case is nothing at all,
/// because only the `joinServer` payload is ever large and everything after it is a number.
///
/// A server that still exceeds it fails honestly: that connection dies, is retried, and is reported
/// (`Error::TooLarge`). Anyone who trusts the server can lift the limit for it entirely —
/// `accept_any_size` on the entry — which is the right shape for a bound that exists to protect
/// against servers nobody here controls.
const MAX_FRAME: usize = 16 * 1024 * 1024;

const HANDSHAKE_ID: u32 = 1;
const JOIN_ID: u32 = 2;
const DMS_ID: u32 = 3;
const PLUGIN_DATA_ID: u32 = 4;
const READ_STATE_ID: u32 = 10;
const READ_STATE_UPDATE_ID: u32 = 12;
const MESSAGE_ID: u32 = 11;

const READ_STATE_PATH: &str = "channels.onReadStateDelta";

/// The *other* read-state subscription, and the one that says a channel was **read**.
///
/// Sharkord publishes two different events and they go to two different subscriptions:
/// `CHANNEL_READ_STATES_DELTA` carries "one more unread here" for an arriving message, and
/// `CHANNEL_READ_STATES_UPDATE` carries the recomputed count when `channels.markAsRead` runs —
/// deliberately, so that "the caller's other sessions need to drop the unread badge too".
///
/// Shiver subscribed only to the first. Everything else was in place to act on a read and none of
/// it ever ran, because the event was never asked for: **six builds** went out trying to fix a
/// badge that would not clear, and this one line is why.
const READ_STATE_UPDATE_PATH: &str = "channels.onReadStateUpdate";
const MESSAGE_PATH: &str = "messages.onNew";

/// One direct-message conversation on one server.
///
/// The name is resolved here rather than carried as a user id, because the rail that shows this is
/// drawn inside some *other* server's page and cannot be handed a lookup table of that server's
/// users to resolve it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMessage {
    pub channel_id: i64,
    pub user_name: String,
    /// when the last message in this conversation was sent, in milliseconds.
    ///
    /// The server's own answer, not Shiver's observation — `dms.get` computes it as
    /// `max(messages.createdAt)` per channel and already sorts its reply by it
    /// (`db/queries/dms.ts`). So it covers the whole history rather than only what Shiver has been
    /// awake to see, which is the difference between a conversation list ordered usefully and one
    /// ordered by when Shiver happened to be running.
    ///
    /// `None` only from a server that answers without the field.
    pub last_message_at: Option<u64>,
}

/// What the server says about itself when Shiver joins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Joined {
    /// `serverId` as the server reports it, so Shiver can tell it is the server it meant to reach
    pub server_id: Option<String>,
    /// this user, so a message Shiver's own account wrote is not announced back to them
    pub own_user_id: Option<i64>,
    /// unread count per channel id, the baseline every later delta is applied to
    pub read_states: HashMap<i64, u32>,
    /// which channels are direct messages, so a dm can be told from a channel
    pub dm_channels: Vec<i64>,
    /// channel id -> name, so a message can say where it came from
    pub channel_names: HashMap<i64, String>,
    /// user id -> name, so a message can say who sent it.
    ///
    /// The whole list, which on a large server is tens of thousands of people and a few hundred
    /// kilobytes. Kept because any of them may be the next to post, and the join payload is the
    /// only place these names arrive.
    pub user_names: HashMap<i64, String>,
    /// this user's conversations on this server, for Shiver's own direct-message list
    pub dms: Vec<DirectMessage>,
    /// The unread floor this user last stored on **this server**, shared across their devices.
    ///
    /// Kept in the companion plugin's per-user storage, which is what makes it shared: the plugin
    /// holds it server-side against the user's own account, so the phone and the desktop measure
    /// their badges from the same place. Without the plugin each device keeps its own floor and the
    /// two drift — read a server on one and the other goes on claiming those messages are unread.
    ///
    /// `None` where the plugin is absent, or where it has nothing stored yet.
    pub shared_floor: Option<HashMap<i64, u32>>,
    /// the version of Shiver's companion plugin on this server, if it is installed.
    ///
    /// Read from the join payload's `pluginsMetadata`, which costs nothing extra: it arrives with
    /// everything else. Worth knowing because it is **not** otherwise askable as an ordinary user —
    /// `plugins.get`, the obvious way to list them, needs `MANAGE_PLUGINS` and so answers only an
    /// admin, and `/info` does not mention plugins at all.
    ///
    /// The version rather than a flag, because the features that depend on this plugin depend on
    /// particular versions of it, and "installed" has already once meant "installed but too old to
    /// do the thing being asked of it".
    pub plugin_version: Option<String>,
}

/// A message that arrived on a server Shiver is watching but not showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub channel_id: i64,
    /// absent for a message a plugin posted, which has a plugin id instead of a person
    pub user_id: Option<i64>,
    pub plugin_id: Option<String>,
    /// the message as text, with Sharkord's html taken off
    pub text: String,
}

/// Something Shiver learned from a server it is not showing.
///
/// One variant so far. It is an enum rather than a struct because the same connection is where
/// message previews and direct messages would arrive from, and those are the next things Shiver
/// wants from a server it is not looking at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// one channel's unread count moved by `delta`
    Unread { channel_id: i64, delta: i64 },
    /// one channel's unread count **is now** `count`, which is how a read arrives.
    ///
    /// Sharkord sends two shapes down the same subscription and they mean opposite things. A new
    /// message publishes a `delta` to add; `channels.markAsRead` publishes the recomputed `count`
    /// for that channel — deliberately, "the caller's other sessions need to drop the unread badge
    /// too" (`routers/channels/mark-as-read.ts`). Read as a delta it is nonsense, and read as
    /// nothing at all — which is what Shiver did — a channel read on one device left the badge
    /// standing on every other one until the socket happened to reconnect.
    UnreadSet { channel_id: i64, count: u32 },
    /// somebody posted, and Shiver has the message itself rather than only a count
    Posted(NewMessage),
}

/* ─────────────────────────── the wire format ─────────────────────────── */

/// The websocket address for a server's origin.
///
/// `?connectionParams=1` is not optional: it is how tRPC is told to wait for a params message
/// before building the connection context. Getting this wrong does not look like an auth failure,
/// it looks like the server crashing mid-upgrade.
pub fn ws_url(origin: &str) -> Result<String> {
    let trimmed = origin.trim_end_matches('/');

    // wss only. This socket carries the session token in its first frame, so a `ws://` version of
    // it would put a live credential on the wire in the clear — the same reason Shiver refuses to
    // add an http server at all.
    let Some(rest) = trimmed.strip_prefix("https://") else {
        return Err(Error::InvalidOrigin(format!(
            "'{origin}' is not an https address, and Shiver will not carry a session over ws://"
        )));
    };

    Ok(format!("wss://{rest}/?connectionParams=1"))
}

/// The first frame on a new socket, which authenticates it.
pub fn params_frame(token: &str) -> String {
    serde_json::json!({ "method": "connectionParams", "data": { "token": token } }).to_string()
}

/// One tRPC request. `method` is `query` or `subscription`.
pub fn request_frame(id: u32, method: &str, path: &str, input: Value) -> String {
    serde_json::json!({
        "id": id,
        "jsonrpc": "2.0",
        "method": method,
        "params": { "path": path, "input": input },
    })
    .to_string()
}

/// A frame from the server, reduced to the three things Shiver acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// a reply to the request with this id, carrying its payload
    Data { id: Option<u64>, data: Value },
    /// a subscription confirming it is running
    Started { id: Option<u64> },
    /// the server refused something, with a message already written for a person to read
    Failed { id: Option<u64>, message: String },
    /// keepalives, reconnect notifications and anything a later Sharkord adds
    Other,
}

pub fn parse_reply(text: &str) -> Reply {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Reply::Other;
    };

    let id = value.get("id").and_then(Value::as_u64);

    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("The server refused the request");

        return Reply::Failed {
            id,
            message: message.to_string(),
        };
    }

    let Some(result) = value.get("result") else {
        return Reply::Other;
    };

    match result.get("type").and_then(Value::as_str) {
        Some("started") => Reply::Started { id },
        // `data` is the type for both a query's answer and a subscription's emission
        _ => match result.get("data") {
            Some(data) => Reply::Data {
                id,
                data: data.clone(),
            },
            None => Reply::Other,
        },
    }
}

/// Reads the parts of `others.joinServer`'s answer that Shiver uses.
///
/// Tolerant on purpose: a Sharkord that renames a field Shiver does not need must not stop the
/// connection working, and one that drops `readStates` should leave Shiver showing no unread rather
/// than refusing to connect.
pub fn parse_join(data: &Value) -> Joined {
    let read_states = data
        .get("readStates")
        .and_then(Value::as_object)
        .map(|states| {
            states
                .iter()
                .filter_map(|(channel, count)| {
                    // the keys arrive as strings because json objects have no integer keys
                    Some((channel.parse::<i64>().ok()?, as_count(count)?))
                })
                .collect()
        })
        .unwrap_or_default();

    let dm_channels = data
        .get("channels")
        .and_then(Value::as_array)
        .map(|channels| {
            channels
                .iter()
                .filter(|channel| channel.get("isDm").and_then(Value::as_bool) == Some(true))
                .filter_map(|channel| channel.get("id").and_then(Value::as_i64))
                .collect()
        })
        .unwrap_or_default();

    let channel_names = data
        .get("channels")
        .and_then(Value::as_array)
        .map(|channels| {
            channels
                .iter()
                .filter_map(|channel| {
                    Some((
                        channel.get("id").and_then(Value::as_i64)?,
                        channel.get("name").and_then(Value::as_str)?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    let user_names = data
        .get("users")
        .and_then(Value::as_array)
        .map(|users| {
            users
                .iter()
                .filter_map(|user| {
                    Some((
                        user.get("id").and_then(Value::as_i64)?,
                        user.get("name").and_then(Value::as_str)?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    Joined {
        server_id: data
            .get("serverId")
            .and_then(Value::as_str)
            .map(str::to_string),
        own_user_id: data.get("ownUserId").and_then(Value::as_i64),
        read_states,
        dm_channels,
        channel_names,
        user_names,
        plugin_version: plugin_version(data, SHIVER_PLUGIN_ID),
        // asked for separately, after the join, by the query that can reach the plugin's storage
        shared_floor: None,
        // filled in after the join, by the query that knows who each conversation is with
        dms: Vec::new(),
    }
}

/// The id Shiver's companion plugin installs under.
pub const SHIVER_PLUGIN_ID: &str = "shiver";

/// The installed version of one plugin, from a join payload's `pluginsMetadata`.
///
/// Separated and pure so it can be tested against the shape Sharkord actually sends
/// (`TPluginMetadata`: `pluginId`, `name`, `description`, `version`).
fn plugin_version(data: &Value, plugin_id: &str) -> Option<String> {
    data.get("pluginsMetadata")?
        .as_array()?
        .iter()
        .find(|plugin| plugin.get("pluginId").and_then(Value::as_str) == Some(plugin_id))
        .and_then(|plugin| plugin.get("version").and_then(Value::as_str))
        .map(str::to_string)
}

/// A `messages.onNew` emission, if that is what this frame is.
pub fn parse_message(data: &Value) -> Option<Event> {
    let channel_id = data.get("channelId").and_then(Value::as_i64)?;

    Some(Event::Posted(NewMessage {
        channel_id,
        user_id: data.get("userId").and_then(Value::as_i64),
        plugin_id: data
            .get("pluginId")
            .and_then(Value::as_str)
            .map(str::to_string),
        text: plain_text(data.get("content").and_then(Value::as_str).unwrap_or("")),
    }))
}

/// Sharkord's message html as the one line a notification can show.
///
/// Tags come off, the handful of entities its editor emits are put back, and runs of whitespace
/// collapse — so `<p>hello</p><p>there</p>` reads "hello there" rather than "hellothere". Not a
/// sanitiser: nothing here is ever put back into a page, it is text for a notification.
pub fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut inside_tag = false;

    for character in html.chars() {
        match character {
            '<' => inside_tag = true,
            // a tag is a word boundary: two paragraphs are two words rather than one run-on
            '>' => {
                inside_tag = false;

                text.push(' ');
            }
            _ if !inside_tag => text.push(character),
            _ => {}
        }
    }

    let text = text
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        // last, so an escaped ampersand cannot revive one of the entities above
        .replace("&amp;", "&");

    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads this user's conversations, and puts a name to each of them.
///
/// Two pieces from two places. `dms.get` answers with pairs of channel and user id, and the names
/// live in the `users` list the join already returned — so no second round trip for those, which
/// matters: a server can carry tens of thousands of users, and Shiver wants the handful it is
/// actually in a conversation with.
async fn fetch_dms(socket: &mut Socket, joined: &Value) -> Result<Vec<DirectMessage>> {
    // A void procedure, and strict about it: `"input": null` is refused with
    // `expected "void", received null`, so the key has to be absent rather than empty.
    send(socket, void_frame(DMS_ID, "dms.get")).await?;

    let conversations = await_reply(socket, DMS_ID, "dms.get").await?;

    let Some(conversations) = conversations.as_array() else {
        eprintln!("[shiver] dms.get did not answer with a list, so no conversations were read");

        return Ok(Vec::new());
    };

    let parsed = parse_dms(conversations, joined);

    // Counted out loud, because this is the one step here that fails **silently**: every field in
    // `parse_dms` is read with `and_then`, so a payload whose shape has moved yields a shorter list
    // rather than an error, and an empty conversation list looks exactly like having no
    // conversations. The two numbers disagreeing is the whole diagnosis.
    if parsed.len() != conversations.len() {
        eprintln!(
            "[shiver] dms.get returned {} conversation(s) and {} could be read — the rest are              missing a userId, or a name for it in the join payload",
            conversations.len(),
            parsed.len()
        );
    }

    Ok(parsed)
}

/// Reads this user's shared unread floor out of the companion plugin's storage.
///
/// `plugins.getUserData` is a **query** and needs only `USE_PLUGINS`, so an ordinary member can ask
/// and this connection stays read-only — which matters, because that is a property of this whole
/// module rather than an accident. The matching write is not done here and cannot be: it is a
/// mutation, and it is performed by the bridge from inside the server's own page, where Shiver
/// already writes the muted-channel list.
///
/// Failing costs the shared floor and nothing else. Every caller falls back to its local one.
async fn fetch_shared_floor(socket: &mut Socket) -> Result<Option<HashMap<i64, u32>>> {
    let stored = call(
        socket,
        PLUGIN_DATA_ID,
        "plugins.getUserData",
        serde_json::json!({ "pluginId": SHIVER_PLUGIN_ID }),
    )
    .await?;

    Ok(parse_shared_floor(&stored))
}

/// The reading of that answer, separated from the asking of it.
///
/// Json object keys are strings, so the channel ids arrive as `"12"` rather than `12` and have to
/// be parsed back. A key that is not a number, or a count that is not one, is skipped rather than
/// failing the lot: this row is written by Shiver but it is stored on somebody else's server.
fn parse_shared_floor(stored: &Value) -> Option<HashMap<i64, u32>> {
    let floor = stored.get("readFloor")?.as_object()?;

    Some(
        floor
            .iter()
            .filter_map(|(channel_id, count)| {
                Some((channel_id.parse::<i64>().ok()?, count.as_u64()? as u32))
            })
            .collect(),
    )
}

/// The reading of that answer, separated from the asking of it.
///
/// Pure, so it can be tested — which matters more than it looks: every field here is read with
/// `and_then`, so a shape that is not what Shiver expects produces a shorter list or a missing
/// timestamp rather than an error. A dropped `lastMessageAt` in particular is invisible at runtime,
/// since the list still appears and is merely in the wrong order.
fn parse_dms(conversations: &[Value], joined: &Value) -> Vec<DirectMessage> {
    let users = joined
        .get("users")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let wanted: std::collections::HashSet<i64> = conversations
        .iter()
        .filter_map(|dm| dm.get("userId").and_then(Value::as_i64))
        .collect();

    let names: HashMap<i64, String> = users
        .iter()
        .filter_map(|user| {
            let id = user.get("id").and_then(Value::as_i64)?;

            if !wanted.contains(&id) {
                return None;
            }

            Some((id, user.get("name").and_then(Value::as_str)?.to_string()))
        })
        .collect();

    conversations
        .iter()
        .filter_map(|dm| {
            let channel_id = dm.get("channelId").and_then(Value::as_i64)?;
            let user_id = dm.get("userId").and_then(Value::as_i64)?;

            Some(DirectMessage {
                channel_id,
                // a conversation with someone no longer in the user list still exists, and saying
                // so is better than dropping it out of the list without explanation
                user_name: names
                    .get(&user_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown".into()),
                // Read as f64 first and rounded. A webview hands numbers back as doubles and
                // serde_json writes a large one in exponential form, which `as_u64` refuses
                // outright — the same trap desktop's `optional_millis` exists for. This value
                // arrives straight off the socket rather than through a page, so `as_u64` would
                // very likely do; taking both costs nothing and cannot be the reason a
                // conversation list comes back unsorted.
                last_message_at: dm.get("lastMessageAt").and_then(|at| {
                    at.as_u64()
                        .or_else(|| at.as_f64().filter(|at| *at >= 0.0).map(|at| at as u64))
                }),
            })
        })
        .collect()
}

/// A request frame for a procedure that takes no input at all.
pub fn void_frame(id: u32, path: &str) -> String {
    serde_json::json!({
        "id": id,
        "jsonrpc": "2.0",
        "method": "query",
        "params": { "path": path },
    })
    .to_string()
}

/// Counts arrive as integers, but a webview or a future server may hand back a double.
fn as_count(value: &Value) -> Option<u32> {
    if let Some(number) = value.as_u64() {
        return Some(number.min(u32::MAX as u64) as u32);
    }

    let number = value.as_f64()?;

    if !number.is_finite() || number < 0.0 {
        return None;
    }

    Some(number.round().min(u32::MAX as f64) as u32)
}

/// A read-state delta, if that is what this frame is.
pub fn parse_delta(data: &Value) -> Option<Event> {
    let channel_id = data.get("channelId").and_then(Value::as_i64)?;

    // `delta` first, because it is the common one: every arriving message is one of these. A frame
    // carrying `count` instead is a read — see `Event::UnreadSet`.
    if let Some(delta) = data.get("delta").and_then(Value::as_i64) {
        return Some(Event::Unread { channel_id, delta });
    }

    let count = data.get("count").and_then(Value::as_u64)?;

    Some(Event::UnreadSet {
        channel_id,
        count: count.min(u64::from(u32::MAX)) as u32,
    })
}

/// Sets one channel's unread count outright, for a read reported by another of this user's devices.
pub fn set_unread(read_states: &mut HashMap<i64, u32>, channel_id: i64, count: u32) {
    if count == 0 {
        read_states.remove(&channel_id);
    } else {
        read_states.insert(channel_id, count);
    }
}

/// What Shiver shows on a server's tile: what arrived while Shiver was watching, minus muted channels.
///
/// Counted against a baseline rather than absolutely, and the difference is the whole point.
/// Sharkord's read state is every message the user has not opened the channel to read, so on a
/// public server it is the entire backlog — a tile that says 99+ from the first launch, says it
/// forever, and means nothing. The desktop client never had this problem because its badge counts
/// *notifications*, and there is no notification for a message posted before Shiver existed.
///
/// So the counts at the moment Shiver connects are the floor, and the badge is what has arrived
/// since. A channel that goes down — the user read it somewhere — contributes nothing rather than a
/// negative, so reading one channel cannot hide new messages in another.
///
/// Muting is Shiver's, not the server's — the server keeps counting a muted channel and is right to,
/// because the mute belongs to this user on this device. So the filter is applied here, at the last
/// moment, rather than by asking the server for less.
pub fn unread_total(
    read_states: &HashMap<i64, u32>,
    baseline: &HashMap<i64, u32>,
    muted: &[i64],
) -> u32 {
    read_states
        .iter()
        .filter(|(channel, _)| !muted.contains(channel))
        .map(|(channel, count)| count.saturating_sub(baseline.get(channel).copied().unwrap_or(0)))
        .sum()
}

/// Applies a delta to a channel's count, never below zero.
pub fn apply_delta(read_states: &mut HashMap<i64, u32>, channel_id: i64, delta: i64) {
    let entry = read_states.entry(channel_id).or_insert(0);
    let next = i64::from(*entry).saturating_add(delta);

    *entry = next.max(0).min(u32::MAX as i64) as u32;
}

/* ───────────────────────────── the connection ───────────────────────────── */

/// Tells "this server said more than Shiver accepts" from every other socket failure.
///
/// Only tungstenite's `Capacity` means the limit was hit; the rest are networks and closed sockets,
/// which come right by themselves. The message is the library's own — it names both sizes — and is
/// kept because the numbers are the whole point of the report.
fn oversize_or_unreachable(error: tokio_tungstenite::tungstenite::Error) -> Error {
    use tokio_tungstenite::tungstenite::error::CapacityError;
    use tokio_tungstenite::tungstenite::Error as Tungstenite;

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

/// A live, joined connection to one server.
pub struct Session {
    socket: Socket,
    pub joined: Joined,
}

/// Connects, authenticates, joins and subscribes.
///
/// Returns once the server has answered `joinServer`, so a caller that gets a `Session` back has a
/// connection that is already reporting real unread counts.
pub async fn open(origin: &str, token: &str, accept_any_size: bool) -> Result<Session> {
    let url = ws_url(origin)?;

    // Bounded rather than default: see `MAX_FRAME`. Both limits are set, because `max_frame_size`
    // alone still allows a message assembled from many frames.
    //
    // `None` is tungstenite's "no limit", and it is only ever reached because someone said this
    // server may have it. That is a real choice with a real cost — an unbounded read buffer on a
    // phone — so it is made per server, by the person who knows whose server it is.
    let cap = if accept_any_size {
        None
    } else {
        Some(MAX_FRAME)
    };

    let limits = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: cap,
        max_frame_size: cap,
        ..Default::default()
    };

    let (mut socket, _) = tokio::time::timeout(
        STEP_TIMEOUT,
        tokio_tungstenite::connect_async_with_config(&url, Some(limits), false),
    )
    .await
    .map_err(|_| Error::Unreachable(origin.to_string()))?
    .map_err(|_| Error::Unreachable(origin.to_string()))?;

    send(&mut socket, params_frame(token)).await?;

    let handshake = call(&mut socket, HANDSHAKE_ID, "others.handshake", Value::Null).await?;

    let hash = handshake
        .get("handshakeHash")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::NotSharkord(origin.to_string()))?
        .to_string();

    let joined = call(
        &mut socket,
        JOIN_ID,
        "others.joinServer",
        serde_json::json!({ "handshakeHash": hash }),
    )
    .await?;

    let mut parsed = parse_join(&joined);

    // Asked for separately because the join payload does not carry it: a conversation is a pairing
    // of this user with another, and the server keeps that list of its own. Failing to get it costs
    // the direct-message list and nothing else, so it must not take the connection down with it —
    // unread counting is the job this connection exists for.
    match fetch_dms(&mut socket, &joined).await {
        Ok(dms) => parsed.dms = dms,
        Err(error) => eprintln!("[shiver] could not read {origin}'s direct messages: {error}"),
    }

    // Only where the plugin is actually installed. Asking a server that has none is a round trip
    // whose answer is always nothing, on every connection to every such server.
    if parsed.plugin_version.is_some() {
        match fetch_shared_floor(&mut socket).await {
            Ok(floor) => parsed.shared_floor = floor,
            Err(error) => {
                eprintln!("[shiver] could not read {origin}'s shared unread floor: {error}")
            }
        }
    }

    let joined = parsed;

    send(
        &mut socket,
        request_frame(READ_STATE_ID, "subscription", READ_STATE_PATH, Value::Null),
    )
    .await?;

    // and the one that reports a channel being read, which is a separate subscription entirely
    send(
        &mut socket,
        request_frame(
            READ_STATE_UPDATE_ID,
            "subscription",
            READ_STATE_UPDATE_PATH,
            Value::Null,
        ),
    )
    .await?;

    // The messages themselves, not only the counts. A badge can say a server has three unread; only
    // this can say who wrote them and what they said, which is what a notification needs.
    send(
        &mut socket,
        request_frame(MESSAGE_ID, "subscription", MESSAGE_PATH, Value::Null),
    )
    .await?;

    Ok(Session { socket, joined })
}

impl Session {
    /// The next thing the server has to say, or `None` when the connection has ended.
    ///
    /// Frames Shiver does not understand are skipped rather than ending the loop: keepalives and
    /// whatever a later Sharkord adds are not errors.
    pub async fn next_event(&mut self) -> Option<Event> {
        while let Some(frame) = self.socket.next().await {
            let Ok(Message::Text(text)) = frame else {
                // a close, a binary frame, or a broken socket all mean this connection is over
                match frame {
                    Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_)) => continue,
                    // Said out loud, because this is the one place a socket dies without anyone
                    // being told why: the caller sees the stream end and retries, which looks the
                    // same whether the server closed politely or sent something Shiver refused.
                    Err(error) => {
                        eprintln!("[shiver] a server's socket failed mid-session: {error}");

                        return None;
                    }
                    Ok(_) => return None,
                }
            };

            // tRPC's own keepalive, above the websocket's: the server sends the bare word `PING`
            // every thirty seconds and *terminates* the connection if `PONG` does not come back
            // within five (`applyWSSHandler({ keepAlive: … })` in `utils/wss.ts`). Shiver never
            // answered, so every background socket died about thirty-five seconds after it
            // connected and came back thirty seconds later — unread counts arrived in bursts with
            // half-minute holes between them, which is exactly the kind of fault that reads as "the
            // server is quiet" rather than as a bug.
            if text == "PING" {
                if self
                    .socket
                    .send(Message::Text("PONG".to_string()))
                    .await
                    .is_err()
                {
                    return None;
                }

                continue;
            }

            match parse_reply(&text) {
                Reply::Data { id, data } if id == Some(u64::from(READ_STATE_ID)) => {
                    if let Some(event) = parse_delta(&data) {
                        return Some(event);
                    }
                }
                Reply::Data { id, data } if id == Some(u64::from(READ_STATE_UPDATE_ID)) => {
                    if let Some(event) = parse_delta(&data) {
                        return Some(event);
                    }
                }
                Reply::Data { id, data } if id == Some(u64::from(MESSAGE_ID)) => {
                    if let Some(event) = parse_message(&data) {
                        return Some(event);
                    }
                }
                // Said out loud: a subscription the server refuses is silent otherwise — the socket
                // stays up, nothing arrives, and the server simply looks quiet.
                Reply::Failed { id, message } => {
                    eprintln!("[shiver] the server refused request {id:?}: {message}");
                }
                _ => {}
            }
        }

        None
    }
}

async fn send(socket: &mut Socket, frame: String) -> Result<()> {
    socket
        .send(Message::Text(frame))
        .await
        .map_err(|error| Error::Unreachable(error.to_string()))
}

/// Sends one query and waits for the reply carrying the same id.
async fn call(socket: &mut Socket, id: u32, path: &str, input: Value) -> Result<Value> {
    send(socket, request_frame(id, "query", path, input)).await?;

    await_reply(socket, id, path).await
}

/// Waits for the reply carrying `id`, for a request that has already been sent.
///
/// Split out of `call` so a request built by hand — a void procedure, whose input key has to be
/// absent rather than null — can wait the same way rather than growing a second copy of this loop.
async fn await_reply(socket: &mut Socket, id: u32, path: &str) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());

        let frame = tokio::time::timeout(remaining, socket.next())
            .await
            .map_err(|_| Error::Unreachable(format!("{path} did not answer")))?
            .ok_or_else(|| Error::Unreachable(format!("{path}: the connection closed")))?
            .map_err(oversize_or_unreachable)?;

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
            // the server volunteers frames of its own; none of them answer this call
            _ => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_websocket_url_keeps_the_connection_params_query() {
        assert_eq!(
            ws_url("https://chat.example.com").unwrap(),
            "wss://chat.example.com/?connectionParams=1"
        );
        assert_eq!(
            ws_url("https://localhost:4991/").unwrap(),
            "wss://localhost:4991/?connectionParams=1"
        );
    }

    /// The first frame on this socket is the session token, so plain ws is never built — not for
    /// localhost either.
    #[test]
    fn only_https_becomes_a_websocket_url() {
        assert!(ws_url("http://chat.example.com").is_err());
        assert!(ws_url("http://localhost:4991").is_err());
        assert!(ws_url("ftp://example.com").is_err());
    }

    /// The exact frame the server refused to work without.
    #[test]
    fn the_first_frame_carries_the_token() {
        let frame: Value = serde_json::from_str(&params_frame("abc")).unwrap();

        assert_eq!(frame["method"], "connectionParams");
        assert_eq!(frame["data"]["token"], "abc");
    }

    #[test]
    fn a_request_names_its_path_and_id() {
        let frame: Value =
            serde_json::from_str(&request_frame(7, "query", "others.handshake", Value::Null))
                .unwrap();

        assert_eq!(frame["id"], 7);
        assert_eq!(frame["method"], "query");
        assert_eq!(frame["params"]["path"], "others.handshake");
    }

    #[test]
    fn a_data_reply_carries_its_payload() {
        let text = r#"{"id":1,"result":{"type":"data","data":{"handshakeHash":"h"}}}"#;

        match parse_reply(text) {
            Reply::Data { id, data } => {
                assert_eq!(id, Some(1));
                assert_eq!(data["handshakeHash"], "h");
            }
            other => panic!("expected data, got {other:?}"),
        }
    }

    #[test]
    fn a_started_subscription_is_not_data() {
        assert_eq!(
            parse_reply(r#"{"id":10,"result":{"type":"started"}}"#),
            Reply::Started { id: Some(10) }
        );
    }

    /// The refusal Shiver actually met before it learned to call `joinServer` first. Its message is
    /// the server's own words, so it is worth keeping rather than replacing.
    #[test]
    fn a_refusal_keeps_the_servers_own_message() {
        let text = r#"{"id":10,"error":{"message":"You must be authenticated to perform this action.","code":-32001}}"#;

        assert_eq!(
            parse_reply(text),
            Reply::Failed {
                id: Some(10),
                message: "You must be authenticated to perform this action.".into()
            }
        );
    }

    #[test]
    fn unreadable_frames_are_not_errors() {
        assert_eq!(parse_reply("not json"), Reply::Other);
        assert_eq!(
            parse_reply(r#"{"method":"reconnectNotification"}"#),
            Reply::Other
        );
    }

    /// Shaped on what a live server actually returned, keys as strings and all.
    #[test]
    fn the_join_payload_yields_read_states_and_dms() {
        let data = serde_json::json!({
            "serverId": "019c1482",
            "readStates": { "1": 0, "8": 3, "140": 2 },
            "channels": [
                { "id": 1, "isDm": false, "name": "README" },
                { "id": 9, "isDm": true, "name": "someone" }
            ]
        });

        let joined = parse_join(&data);

        assert_eq!(joined.server_id.as_deref(), Some("019c1482"));
        assert_eq!(joined.read_states.get(&8), Some(&3));
        assert_eq!(joined.read_states.get(&140), Some(&2));
        assert_eq!(joined.dm_channels, vec![9]);
    }

    /// `dms.get`'s answer, as `db/queries/dms.ts` builds it. The timestamp is the field the whole
    /// conversation order rests on, and losing it is invisible — the list still appears, merely in
    /// the wrong order — so it is asserted rather than assumed.
    #[test]
    fn a_conversation_carries_its_name_and_its_latest_message() {
        let conversations = serde_json::json!([
            { "channelId": 9, "userId": 2, "unreadCount": 0, "lastMessageAt": 1_757_000_000_000u64 }
        ]);
        let joined = serde_json::json!({
            "users": [{ "id": 2, "name": "Smiddy" }, { "id": 3, "name": "Someone else" }]
        });

        let dms = parse_dms(conversations.as_array().expect("an array"), &joined);

        assert_eq!(
            dms,
            vec![DirectMessage {
                channel_id: 9,
                user_name: "Smiddy".into(),
                last_message_at: Some(1_757_000_000_000),
            }]
        );
    }

    /// A webview hands numbers back as doubles and serde_json writes a large one in exponential
    /// form, which `as_u64` refuses outright — the trap desktop's `optional_millis` exists for.
    #[test]
    fn a_timestamp_in_exponential_form_is_still_a_timestamp() {
        let conversations = serde_json::json!([
            { "channelId": 9, "userId": 2, "lastMessageAt": 1.757e12 }
        ]);

        let dms = parse_dms(
            conversations.as_array().expect("an array"),
            &serde_json::json!({}),
        );

        assert_eq!(dms[0].last_message_at, Some(1_757_000_000_000));
    }

    /// A conversation with someone no longer in the user list still exists, and a server that
    /// answers without a timestamp still has conversations — neither is a reason to drop the row.
    #[test]
    fn a_conversation_survives_a_missing_name_or_timestamp() {
        let conversations = serde_json::json!([{ "channelId": 9, "userId": 404 }]);

        let dms = parse_dms(
            conversations.as_array().expect("an array"),
            &serde_json::json!({ "users": [] }),
        );

        assert_eq!(dms[0].user_name, "Unknown");
        assert_eq!(dms[0].last_message_at, None);
    }

    #[test]
    fn a_join_payload_missing_everything_still_parses() {
        let joined = parse_join(&serde_json::json!({}));

        assert_eq!(joined, Joined::default());
    }

    #[test]
    fn a_delta_frame_becomes_an_unread_event() {
        let data = serde_json::json!({ "channelId": 8, "delta": 1 });

        assert_eq!(
            parse_delta(&data),
            Some(Event::Unread {
                channel_id: 8,
                delta: 1
            })
        );
        assert_eq!(parse_delta(&serde_json::json!({ "channelId": 8 })), None);
    }

    /// The frame this was written against, captured from demo.sharkord.com rather than imagined:
    /// `messages.onNew` emits the message itself, and its content is html.
    #[test]
    fn a_new_message_is_read_from_the_frame_the_server_sends() {
        let data = serde_json::json!({
            "id": 13723,
            "content": "<p>shape check for messages.onNew</p>",
            "userId": 52737,
            "pluginId": null,
            "channelId": 8,
            "createdAt": 1788726108661i64,
            "files": [],
        });

        assert_eq!(
            parse_message(&data),
            Some(Event::Posted(NewMessage {
                channel_id: 8,
                user_id: Some(52737),
                plugin_id: None,
                text: "shape check for messages.onNew".into(),
            }))
        );

        // a frame with no channel is not a message Shiver can place, and must not panic
        assert_eq!(parse_message(&serde_json::json!({ "content": "hi" })), None);
    }

    /// The body of a notification, so what lands on the shade is what was written rather than
    /// markup — and two paragraphs read as two words rather than one run-on.
    #[test]
    fn message_html_becomes_the_line_a_person_reads() {
        assert_eq!(plain_text("<p>hello</p>"), "hello");
        assert_eq!(plain_text("<p>hello</p><p>there</p>"), "hello there");
        assert_eq!(plain_text("<p>tom &amp; jerry &lt;3</p>"), "tom & jerry <3");
        assert_eq!(plain_text("<p>  spaced   out  </p>"), "spaced out");
        // an attachment with no words at all, which the caller turns into its own line
        assert_eq!(plain_text("<p></p>"), "");
        // an escaped entity must not be revived into a tag by the unescaping itself
        assert_eq!(plain_text("&amp;lt;p&amp;gt;"), "&lt;p&gt;");
    }

    /// Muting is Shiver's and the server keeps counting, so the filter has to be here or a muted
    /// channel would still light the rail.
    #[test]
    fn muted_channels_are_left_out_of_the_total() {
        let states = HashMap::from([(1, 2), (2, 5), (3, 1)]);
        let none = HashMap::new();

        assert_eq!(unread_total(&states, &none, &[]), 8);
        assert_eq!(unread_total(&states, &none, &[2]), 3);
        assert_eq!(unread_total(&states, &none, &[1, 2, 3]), 0);
    }

    /// The badge is what arrived while Shiver was watching. A public server's backlog is not news,
    /// and counting it is what made a tile say 99+ from the first launch and never stop.
    #[test]
    fn the_backlog_a_server_was_already_holding_is_not_counted() {
        let baseline = HashMap::from([(1, 40), (2, 900)]);

        // nothing has happened since Shiver connected
        assert_eq!(unread_total(&baseline, &baseline, &[]), 0);

        // two messages in one channel, none in the other
        let states = HashMap::from([(1, 42), (2, 900)]);

        assert_eq!(unread_total(&states, &baseline, &[]), 2);
    }

    /// Reading one channel elsewhere must not eat another channel's news, which summing the
    /// difference of the totals would do.
    #[test]
    fn a_channel_read_elsewhere_subtracts_nothing_from_the_others() {
        let baseline = HashMap::from([(1, 10), (2, 10)]);
        // channel 1 was read somewhere else, channel 2 has three new messages
        let states = HashMap::from([(1, 0), (2, 13)]);

        assert_eq!(unread_total(&states, &baseline, &[]), 3);
    }

    /// A channel created after Shiver connected has no floor, so all of it is new.
    #[test]
    fn a_channel_the_baseline_never_saw_counts_in_full() {
        let baseline = HashMap::from([(1, 5)]);
        let states = HashMap::from([(1, 5), (2, 4)]);

        assert_eq!(unread_total(&states, &baseline, &[]), 4);
    }

    /// The server sends a delta of 1 per message rather than a total, so this accumulates — and
    /// must not underflow when a read state comes back down.
    #[test]
    fn deltas_accumulate_and_never_go_negative() {
        let mut states = HashMap::from([(1, 1)]);

        apply_delta(&mut states, 1, 1);
        apply_delta(&mut states, 2, 1);

        assert_eq!(states.get(&1), Some(&2));
        assert_eq!(states.get(&2), Some(&1));

        apply_delta(&mut states, 1, -5);

        assert_eq!(states.get(&1), Some(&0));
    }
}

#[cfg(test)]
mod plugin_tests {
    use super::{plugin_version, SHIVER_PLUGIN_ID};

    fn payload() -> serde_json::Value {
        serde_json::json!({
            "pluginsMetadata": [
                { "pluginId": "something-else", "name": "Other", "version": "2.0.0" },
                { "pluginId": "shiver", "name": "Shiver", "version": "0.1.0" }
            ]
        })
    }

    #[test]
    fn finds_the_plugin_among_others() {
        assert_eq!(plugin_version(&payload(), SHIVER_PLUGIN_ID), Some("0.1.0".into()));
    }

    #[test]
    fn absent_when_not_installed() {
        let without = serde_json::json!({ "pluginsMetadata": [] });

        assert_eq!(plugin_version(&without, SHIVER_PLUGIN_ID), None);
    }

    #[test]
    fn absent_when_the_server_does_not_mention_plugins() {
        // an older Sharkord, or a payload shape that has moved: not installed, as far as Shiver can
        // tell, which is the safe answer — the features behind it degrade rather than misbehave
        assert_eq!(plugin_version(&serde_json::json!({}), SHIVER_PLUGIN_ID), None);
    }
}

#[cfg(test)]
mod floor_tests {
    use super::parse_shared_floor;

    #[test]
    fn reads_channel_ids_back_out_of_string_keys() {
        let stored = serde_json::json!({ "readFloor": { "12": 3, "40": 0 } });
        let floor = parse_shared_floor(&stored).expect("a floor");

        assert_eq!(floor.get(&12), Some(&3));
        assert_eq!(floor.get(&40), Some(&0));
    }

    #[test]
    fn nothing_stored_is_no_floor() {
        // a plugin row that exists but has only the muted list in it
        let stored = serde_json::json!({ "mutedChannels": [1, 2] });

        assert!(parse_shared_floor(&stored).is_none());
    }

    #[test]
    fn a_row_that_is_not_an_object_is_no_floor() {
        assert!(parse_shared_floor(&serde_json::Value::Null).is_none());
        assert!(parse_shared_floor(&serde_json::json!({ "readFloor": 7 })).is_none());
    }

    #[test]
    fn rubbish_entries_are_skipped_rather_than_failing_the_lot() {
        // stored by Shiver, but held on somebody else's server
        let stored = serde_json::json!({ "readFloor": { "12": 3, "no": 1, "40": "lots" } });
        let floor = parse_shared_floor(&stored).expect("a floor");

        assert_eq!(floor.len(), 1);
        assert_eq!(floor.get(&12), Some(&3));
    }
}

#[cfg(test)]
mod read_state_tests {
    use super::{parse_delta, set_unread, Event};
    use std::collections::HashMap;

    #[test]
    fn a_new_message_is_a_delta() {
        let frame = serde_json::json!({ "channelId": 4, "delta": 1 });

        assert_eq!(
            parse_delta(&frame),
            Some(Event::Unread { channel_id: 4, delta: 1 })
        );
    }

    #[test]
    fn a_read_on_another_device_is_a_count() {
        // what `channels.markAsRead` publishes to the caller's other sessions
        let frame = serde_json::json!({ "channelId": 4, "count": 0 });

        assert_eq!(
            parse_delta(&frame),
            Some(Event::UnreadSet { channel_id: 4, count: 0 })
        );
    }

    #[test]
    fn a_frame_that_is_neither_is_ignored() {
        assert_eq!(parse_delta(&serde_json::json!({ "channelId": 4 })), None);
        assert_eq!(parse_delta(&serde_json::json!({ "delta": 1 })), None);
    }

    #[test]
    fn setting_to_zero_forgets_the_channel() {
        let mut states: HashMap<i64, u32> = [(4, 5)].into_iter().collect();

        set_unread(&mut states, 4, 0);

        assert!(!states.contains_key(&4));
    }

    #[test]
    fn setting_replaces_rather_than_adding() {
        let mut states: HashMap<i64, u32> = [(4, 5)].into_iter().collect();

        set_unread(&mut states, 4, 2);

        assert_eq!(states.get(&4), Some(&2));
    }
}
