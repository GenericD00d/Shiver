//! Deletes the messages an identity posted in a channel.
//!
//! A tidy-up for `postas`: testing the unread badge means putting real messages on a real server,
//! and a shared demo server should not be left holding them.
//!
//!   SHIVER_PASSWORD='…' cargo run --example cleanup -- https://demo.sharkord.com someone 8

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
///     SHIVER_PASSWORD='…' cargo run --example cleanup -- https://chat.example.com <identity> <channel id>
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
    let channel_id: i64 = args.next().expect("channel id").parse()?;

    let client = reqwest::Client::builder()
        .user_agent("Shiver/cleanup")
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

    let ws_url = format!(
        "{}/?connectionParams=1",
        origin
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
            .trim_end_matches('/')
    );

    let (mut socket, _) = tokio_tungstenite::connect_async(&ws_url).await?;

    socket
        .send(Message::Text(
            serde_json::json!({ "method": "connectionParams", "data": { "token": token } })
                .to_string(),
        ))
        .await?;

    let handshake = request(&mut socket, 1, "query", "others.handshake", Value::Null).await?;
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

    let own_user_id = joined
        .pointer("/result/data/ownUserId")
        .and_then(Value::as_i64)
        .ok_or("no ownUserId")?;

    let listed = request(
        &mut socket,
        3,
        "query",
        "messages.get",
        serde_json::json!({ "channelId": channel_id, "limit": 50 }),
    )
    .await?;

    let messages = listed
        .pointer("/result/data/messages")
        .or_else(|| listed.pointer("/result/data/items"))
        .or_else(|| listed.pointer("/result/data"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mine: Vec<i64> = messages
        .iter()
        .filter(|message| message.get("userId").and_then(Value::as_i64) == Some(own_user_id))
        .filter_map(|message| message.get("id").and_then(Value::as_i64))
        .collect();

    println!(
        "[cleanup] {identity} wrote {} of the last {} messages",
        mine.len(),
        messages.len()
    );

    let mut id = 10;

    for message_id in mine {
        id += 1;

        let deleted = request(
            &mut socket,
            id,
            "mutation",
            "messages.delete",
            serde_json::json!({ "messageId": message_id }),
        )
        .await?;

        if let Some(error) = deleted.get("error") {
            println!("[cleanup] {message_id}: {error}");
        } else {
            println!("[cleanup] deleted {message_id}");
        }
    }

    Ok(())
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

        if text == "PING" {
            socket.send(Message::Text("PONG".to_string())).await?;

            continue;
        }

        let value: Value = serde_json::from_str(&text)?;

        if value.get("id").and_then(Value::as_u64) == Some(id as u64) {
            return Ok(value);
        }
    }
}
