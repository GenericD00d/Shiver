//! A throwaway probe for Sharkord's tRPC-over-WebSocket protocol.
//!
//! Not part of the app. It exists so `sharkord.rs` can be written against what a real server
//! actually sends rather than against an assumption about tRPC's wire format.
//!
//! What it established, in order, each step found by being refused at the previous one:
//!   1. `POST /login` -> `{ token }`                              (same as desktop's `login.rs`)
//!   2. connect `wss://host/?connectionParams=1`                  (without the query the server
//!      builds its context immediately, finds no params and throws mid-upgrade)
//!   3. send `{ method: "connectionParams", data: { token } }`     (authenticates the socket)
//!   4. query `others.handshake` -> `{ handshakeHash }`
//!   5. query `others.joinServer` with that hash                  (sets `ctx.authenticated`, and
//!      returns the whole initial state including channel read states)
//!   6. only now do `protectedProcedure` subscriptions work
//!
//! Run it as:
//!   SHIVER_PASSWORD='…' cargo run --example wsprobe -- https://demo.sharkord.com <identity>

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The password, from the environment rather than the command line.
///
/// `argv` is readable by any process on the machine through `ps`, and lands in shell history —
/// which is a poor place for a password even in a developer tool that never ships. `SHIVER_PASSWORD`
/// is read once and is not echoed anywhere.
///
///     SHIVER_PASSWORD='…' cargo run --example wsprobe -- https://chat.example.com <identity>
fn password() -> String {
    std::env::var("SHIVER_PASSWORD").unwrap_or_else(|_| {
        eprintln!(
            "set SHIVER_PASSWORD to the account's password — it is not taken on the command line, \
             where `ps` and the shell's history would both keep a copy"
        );
        std::process::exit(2);
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let origin = args.next().expect("origin");
    let identity = args.next().expect("identity");
    let password = password();

    let client = reqwest::Client::builder()
        .user_agent("Shiver/probe")
        .build()?;

    let body: Value = client
        .post(format!("{origin}/login"))
        .json(&serde_json::json!({ "identity": identity, "password": password }))
        .send()
        .await?
        .json()
        .await?;

    let token = body
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("no token in {body}"))?
        .to_string();

    println!("[probe] signed in, token {} chars", token.len());

    let ws_url = format!(
        "{}/?connectionParams=1",
        origin
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
            .trim_end_matches('/')
    );

    let (mut socket, response) = tokio_tungstenite::connect_async(&ws_url).await?;

    println!("[probe] connected, http {}", response.status());

    socket
        .send(Message::Text(
            serde_json::json!({ "method": "connectionParams", "data": { "token": token } })
                .to_string(),
        ))
        .await?;

    let handshake = request(&mut socket, 1, "query", "others.handshake", Value::Null).await?;

    println!("[probe] handshake -> {}", brief(&handshake));

    let hash = handshake
        .pointer("/result/data/handshakeHash")
        .and_then(Value::as_str)
        .ok_or("no handshakeHash")?
        .to_string();

    let joined = request(
        &mut socket,
        2,
        "query",
        "others.joinServer",
        serde_json::json!({ "handshakeHash": hash }),
    )
    .await?;

    if let Some(error) = joined.get("error") {
        println!("[probe] joinServer refused: {}", brief(error));

        return Ok(());
    }

    // what the initial state actually contains, which is what Shiver would read unread counts from
    if let Some(data) = joined.pointer("/result/data") {
        if let Some(object) = data.as_object() {
            let mut keys: Vec<&String> = object.keys().collect();

            keys.sort();
            println!("[probe] joinServer keys: {keys:?}");

            for key in ["channelsReadStates", "readStates", "channels"] {
                if let Some(value) = object.get(key) {
                    println!("[probe]   {key} = {}", brief(value));
                }
            }
        }
    }

    // what a user looks like in that payload, since the dm list needs a name per user id
    if let Some(users) = joined
        .pointer("/result/data/users")
        .and_then(Value::as_array)
    {
        println!("[probe] users: {} total", users.len());

        if let Some(first) = users.first().and_then(Value::as_object) {
            let mut keys: Vec<&String> = first.keys().collect();

            keys.sort();
            println!("[probe]   user keys: {keys:?}");
        }
    }

    // the conversations themselves, which the join payload does not carry
    // a void procedure: the input key has to be absent, not null. `"input": null` is refused with
    // `expected "void", received null`, which is how this was found.
    socket
        .send(Message::Text(
            serde_json::json!({
                "id": 3,
                "jsonrpc": "2.0",
                "method": "query",
                "params": { "path": "dms.get" },
            })
            .to_string(),
        ))
        .await?;

    let dms = read_reply(&mut socket, 3).await?;

    if let Some(error) = dms.get("error") {
        println!("[probe] dms.get refused: {}", brief(error));
    } else {
        println!("[probe] dms.get -> {}", brief(&dms));
    }

    for (id, path) in [(10u32, "messages.onNew"), (11, "channels.onReadStateDelta")] {
        socket
            .send(Message::Text(
                serde_json::json!({
                    "id": id,
                    "jsonrpc": "2.0",
                    "method": "subscription",
                    "params": { "path": path, "input": Value::Null },
                })
                .to_string(),
            ))
            .await?;
    }

    println!("[probe] subscribed; listening 20s");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());

        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, socket.next()).await {
            Err(_) | Ok(None) => break,
            Ok(Some(Err(error))) => {
                println!("[probe] error: {error}");

                break;
            }
            Ok(Some(Ok(Message::Text(text)))) => println!("[probe] <- {}", cut(&text)),
            Ok(Some(Ok(Message::Close(frame)))) => {
                println!("[probe] <- close {frame:?}");

                break;
            }
            Ok(Some(Ok(other))) => println!("[probe] <- {other:?}"),
        }
    }

    println!("[probe] done");

    Ok(())
}

/// Sends one request and returns the reply carrying the same id, printing anything in between.
/// Waits for the reply carrying `id`, for a request already sent.
async fn read_reply(socket: &mut Socket, id: u32) -> Result<Value, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let frame = tokio::time::timeout(remaining, socket.next())
            .await
            .map_err(|_| format!("reply {id} timed out"))?
            .ok_or_else(|| format!("reply {id}: stream ended"))??;

        let Message::Text(text) = frame else {
            continue;
        };

        let value: Value = serde_json::from_str(&text)?;

        if value.get("id").and_then(Value::as_u64) == Some(id as u64) {
            return Ok(value);
        }
    }
}

async fn request(
    socket: &mut Socket,
    id: u32,
    method: &str,
    path: &str,
    input: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    socket
        .send(Message::Text(
            serde_json::json!({
                "id": id,
                "jsonrpc": "2.0",
                "method": method,
                "params": { "path": path, "input": input },
            })
            .to_string(),
        ))
        .await?;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let frame = tokio::time::timeout(remaining, socket.next())
            .await
            .map_err(|_| format!("{path} timed out"))?
            .ok_or_else(|| format!("{path}: stream ended"))??;

        let Message::Text(text) = frame else {
            continue;
        };

        let value: Value = serde_json::from_str(&text)?;

        if value.get("id").and_then(Value::as_u64) == Some(id as u64) {
            return Ok(value);
        }

        println!("[probe] (while waiting for {path}) <- {}", cut(&text));
    }
}

fn cut(text: &str) -> String {
    if text.len() > 600 {
        format!("{}… [{} bytes]", &text[..600], text.len())
    } else {
        text.to_string()
    }
}

fn brief(value: &Value) -> String {
    cut(&value.to_string())
}
