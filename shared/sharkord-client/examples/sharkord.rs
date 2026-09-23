//! A developer tool for Sharkord's tRPC-over-WebSocket protocol (not shipped).
//!
//!   SHIVER_PASSWORD='…' cargo run -p sharkord-client --example sharkord -- <origin> <identity> probe
//!   SHIVER_PASSWORD='…' cargo run -p sharkord-client --example sharkord -- <origin> <identity> post <channel> [text]
//!   SHIVER_PASSWORD='…' cargo run -p sharkord-client --example sharkord -- <origin> <identity> cleanup <channel>
//!
//! `probe` prints the join payload's shape, `dms.get`, and 20 s of `messages.onNew` /
//! `channels.onReadStateDelta` frames. `post` sends a message as a second person (a user's own
//! messages never count as unread, so badge tests need one); `cleanup` deletes that identity's
//! recent messages in a channel. The password comes from the environment, never argv.
//!
//! The sequence: `POST /login` → connect `wss://host/?connectionParams=1` → send
//! `{method: "connectionParams", data: {token}}` → `others.handshake` → `others.joinServer`; only
//! then do protected procedures and subscriptions work.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::time::{timeout, Instant};
use tokio_tungstenite::tungstenite::Message;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type Failure = Box<dyn std::error::Error>;

const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

struct Connection {
    socket: Socket,
    next_id: u64,
}

impl Connection {
    async fn open(origin: &str, identity: &str, password: &str) -> Result<(Self, Value), Failure> {
        let token = shiver_core::login::sign_in(origin, identity, password).await?;

        let ws_url = format!(
            "{}/?connectionParams=1",
            origin
                .trim_end_matches('/')
                .replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1)
        );

        let (mut socket, _) = tokio_tungstenite::connect_async(&ws_url).await?;

        socket
            .send(Message::Text(
                json!({ "method": "connectionParams", "data": { "token": token } }).to_string(),
            ))
            .await?;

        let mut connection = Connection { socket, next_id: 1 };

        let handshake = connection
            .request("query", "others.handshake", Value::Null)
            .await?;
        let hash = handshake
            .pointer("/result/data/handshakeHash")
            .and_then(Value::as_str)
            .ok_or("no handshakeHash")?
            .to_string();
        let joined = connection
            .request(
                "query",
                "others.joinServer",
                json!({ "handshakeHash": hash }),
            )
            .await?;

        if let Some(error) = joined.get("error") {
            return Err(format!("joinServer refused: {}", cut(&error.to_string())).into());
        }

        Ok((connection, joined))
    }

    async fn send(&mut self, method: &str, path: &str, input: Value) -> Result<u64, Failure> {
        let id = self.next_id;

        self.next_id += 1;

        let frame = json!({ "id": id, "jsonrpc": "2.0", "method": method, "params": { "path": path, "input": input } });

        self.socket.send(Message::Text(frame.to_string())).await?;

        Ok(id)
    }

    /// Sends a request and waits for its reply, answering pings and printing anything else.
    async fn request(&mut self, method: &str, path: &str, input: Value) -> Result<Value, Failure> {
        let id = self.send(method, path, input).await?;
        let deadline = Instant::now() + REPLY_TIMEOUT;

        loop {
            let frame = timeout(
                deadline.saturating_duration_since(Instant::now()),
                self.socket.next(),
            )
            .await
            .map_err(|_| format!("{path} timed out"))?
            .ok_or_else(|| format!("{path}: stream ended"))??;

            let Some(value) = self.frame(frame).await? else {
                continue;
            };

            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(value);
            }

            println!("(while waiting for {path}) <- {}", cut(&value.to_string()));
        }
    }

    /// Parses a text frame; answers `PING`.
    async fn frame(&mut self, frame: Message) -> Result<Option<Value>, Failure> {
        let Message::Text(text) = frame else {
            return Ok(None);
        };

        if text == "PING" {
            self.socket.send(Message::Text("PONG".into())).await?;

            return Ok(None);
        }

        Ok(Some(serde_json::from_str(&text)?))
    }
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let [origin, identity, command, rest @ ..] = args.as_slice() else {
        return Err(
            "usage: sharkord <origin> <identity> probe | post <channel> [text] | cleanup <channel>"
                .into(),
        );
    };

    let password = std::env::var("SHIVER_PASSWORD")
        .map_err(|_| "set SHIVER_PASSWORD to the account's password")?;
    let (mut connection, joined) = Connection::open(origin, identity, &password).await?;
    let channel = || -> Result<i64, Failure> { Ok(rest.first().ok_or("which channel?")?.parse()?) };

    match command.as_str() {
        "probe" => probe(&mut connection, &joined).await,
        "post" => {
            let text = rest
                .get(1)
                .map_or("hello from Shiver's tests", String::as_str);
            let sent = connection
                .request("mutation", "messages.send", json!({ "channelId": channel()?, "content": format!("<p>{text}</p>"), "files": [] }))
                .await?;

            report(&sent, "sent");

            Ok(())
        }
        "cleanup" => cleanup(&mut connection, &joined, channel()?).await,
        other => Err(format!("unknown command {other}").into()),
    }
}

async fn probe(connection: &mut Connection, joined: &Value) -> Result<(), Failure> {
    if let Some(data) = joined.pointer("/result/data").and_then(Value::as_object) {
        let mut keys: Vec<&String> = data.keys().collect();

        keys.sort();
        println!("joinServer keys: {keys:?}");

        for key in ["channelsReadStates", "readStates", "channels"] {
            if let Some(value) = data.get(key) {
                println!("  {key} = {}", cut(&value.to_string()));
            }
        }
    }

    let dms = connection.request("query", "dms.get", Value::Null).await?;

    println!("dms.get -> {}", cut(&dms.to_string()));

    for path in ["messages.onNew", "channels.onReadStateDelta"] {
        connection.send("subscription", path, Value::Null).await?;
    }

    println!("subscribed; listening 20s");

    let deadline = Instant::now() + REPLY_TIMEOUT;

    while let Ok(Some(frame)) = timeout(
        deadline.saturating_duration_since(Instant::now()),
        connection.socket.next(),
    )
    .await
    {
        match frame? {
            Message::Close(frame) => {
                println!("<- close {frame:?}");

                break;
            }
            frame => {
                if let Some(value) = connection.frame(frame).await? {
                    println!("<- {}", cut(&value.to_string()));
                }
            }
        }
    }

    Ok(())
}

async fn cleanup(
    connection: &mut Connection,
    joined: &Value,
    channel_id: i64,
) -> Result<(), Failure> {
    let own_user_id = joined
        .pointer("/result/data/ownUserId")
        .and_then(Value::as_i64)
        .ok_or("no ownUserId")?;
    let listed = connection
        .request(
            "query",
            "messages.get",
            json!({ "channelId": channel_id, "limit": 50 }),
        )
        .await?;

    let messages = [
        "/result/data/messages",
        "/result/data/items",
        "/result/data",
    ]
    .iter()
    .find_map(|pointer| listed.pointer(pointer).and_then(Value::as_array))
    .cloned()
    .unwrap_or_default();

    let mine: Vec<i64> = messages
        .iter()
        .filter(|message| message.get("userId").and_then(Value::as_i64) == Some(own_user_id))
        .filter_map(|message| message.get("id").and_then(Value::as_i64))
        .collect();

    println!(
        "{} of the last {} messages are this identity's",
        mine.len(),
        messages.len()
    );

    for message_id in mine {
        let deleted = connection
            .request(
                "mutation",
                "messages.delete",
                json!({ "messageId": message_id }),
            )
            .await?;

        report(&deleted, &format!("deleted {message_id}"));
    }

    Ok(())
}

fn report(reply: &Value, success: &str) {
    match reply.get("error") {
        Some(error) => println!("refused: {}", cut(&error.to_string())),
        None => println!("{success}"),
    }
}

/// At most 600 characters (on a character boundary).
fn cut(text: &str) -> String {
    match text.char_indices().nth(600) {
        Some((end, _)) => format!("{}… [{} bytes]", &text[..end], text.len()),
        None => text.to_string(),
    }
}
