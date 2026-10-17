//! The wire format: tRPC frames over the websocket, and the payloads Shiver reads out of them.

use std::collections::HashMap;

use serde_json::Value;
use shiver_core::text::presentable;

use crate::{
    html::{mentioned, plain_text},
    Avatar, DirectMessage, Error, Event, Joined, NewMessage, Result,
};

pub(crate) const HANDSHAKE_ID: u32 = 1;

pub(crate) const JOIN_ID: u32 = 2;

pub(crate) const DMS_ID: u32 = 3;

pub(crate) const PLUGIN_DATA_ID: u32 = 4;

pub(crate) const READ_STATE_ID: u32 = 10;

pub(crate) const MESSAGE_ID: u32 = 11;

pub(crate) const READ_STATE_UPDATE_ID: u32 = 12;

/// "One more unread here", published per arriving message.
const READ_STATE_PATH: &str = "channels.onReadStateDelta";

/// "This channel's count is now N", published when the user reads a channel on any device.
const READ_STATE_UPDATE_PATH: &str = "channels.onReadStateUpdate";

const MESSAGE_PATH: &str = "messages.onNew";

/// What a joined socket subscribes to; every event arrives on one of them.
pub(crate) const SUBSCRIPTIONS: [(u32, &str); 3] = [
    (READ_STATE_ID, READ_STATE_PATH),
    (READ_STATE_UPDATE_ID, READ_STATE_UPDATE_PATH),
    (MESSAGE_ID, MESSAGE_PATH),
];

/// The id Shiver's companion plugin installs under.
pub(crate) const SHIVER_PLUGIN_ID: &str = "shiver";

/// tRPC's code for a rate limit: Sharkord allows each user five joins a minute.
const TOO_MANY_REQUESTS: i64 = -32029;

/// The websocket address for an https origin. wss only: the first frame carries the session.
pub(crate) fn ws_url(origin: &str) -> Result<String> {
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

pub(crate) fn params_frame(token: &str) -> String {
    serde_json::json!({ "method": "connectionParams", "data": { "token": token } }).to_string()
}

/// One tRPC request. `input` is omitted entirely when `None` (void procedures refuse `null`).
pub(crate) fn request_frame(id: u32, method: &str, path: &str, input: Option<Value>) -> String {
    let mut params = serde_json::json!({ "path": path });

    if let Some(input) = input {
        params["input"] = input;
    }

    serde_json::json!({ "id": id, "jsonrpc": "2.0", "method": method, "params": params })
        .to_string()
}

/// A frame from the server, reduced to what Shiver acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    Data {
        id: Option<u64>,
        data: Value,
    },
    Started {
        id: Option<u64>,
    },
    /// `busy`: refused for asking too often (`TOO_MANY_REQUESTS`), not for who is asking
    Failed {
        id: Option<u64>,
        message: String,
        busy: bool,
    },
    Other,
}

pub(crate) fn parse_reply(text: &str) -> Reply {
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
        let busy = error.get("code").and_then(Value::as_i64) == Some(TOO_MANY_REQUESTS)
            || error.pointer("/data/code").and_then(Value::as_str) == Some("TOO_MANY_REQUESTS");

        return Reply::Failed { id, message, busy };
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
pub(crate) fn parse_join(data: &Value) -> Joined {
    let mut joined = Joined {
        own_user_id: data.get("ownUserId").and_then(Value::as_i64),
        read_states: data
            .get("readStates")
            .and_then(channel_counts)
            .unwrap_or_default(),
        plugin_version: plugin_version(data, SHIVER_PLUGIN_ID),
        ..Joined::default()
    };

    joined.own_role_ids = joined
        .own_user_id
        .and_then(|own| {
            data.get("users")?
                .as_array()?
                .iter()
                .find(|user| user.get("id").and_then(Value::as_i64) == Some(own))
        })
        .and_then(|user| user.get("roleIds")?.as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .collect();

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

    joined.user_avatars = data
        .get("users")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|user| {
            Some((
                user.get("id").and_then(Value::as_i64)?,
                parse_avatar(user.get("avatar")?)?,
            ))
        })
        .collect();

    joined
}

/// A signed file as Sharkord sends it: `{ name, _accessToken?, _accessTokenExpiresAt? }`.
fn parse_avatar(file: &Value) -> Option<Avatar> {
    Some(Avatar {
        name: file.get("name").and_then(Value::as_str)?.to_string(),
        access_token: file
            .get("_accessToken")
            .and_then(Value::as_str)
            .map(str::to_string),
        expires_at: file.get("_accessTokenExpiresAt").and_then(|at| {
            at.as_u64().or_else(|| {
                at.as_f64()
                    .filter(|at| *at >= 0.0)
                    .map(|at| at.round() as u64)
            })
        }),
    })
}

fn plugin_version(data: &Value, plugin_id: &str) -> Option<String> {
    data.get("pluginsMetadata")?
        .as_array()?
        .iter()
        .find(|plugin| plugin.get("pluginId").and_then(Value::as_str) == Some(plugin_id))?
        .get("version")?
        .as_str()
        .filter(|version| {
            version.len() <= 32
                && version
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || ".+-".contains(character))
        })
        .map(str::to_string)
}

/// A millisecond time from json, accepting a double.
fn as_time(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|at| *at >= 0.0)
            .map(|at| at.round() as u64)
    })
}

/// The conversations the join lists (`directMessages`, Sharkord 0.0.28 and newer), or `None` from an
/// older server, which answers `dms.get` instead.
pub(crate) fn join_dms(data: &Value) -> Option<&[Value]> {
    data.get("directMessages")?.as_array().map(Vec::as_slice)
}

/// A conversation list (the join's, or `dms.get`'s on an older server), named and pictured from the
/// join's user list. Unknown users read as "Unknown". One the user closed is left out until a message
/// arrives after it was closed, as Sharkord's own list does.
pub(crate) fn parse_dms(
    conversations: &[Value],
    joined: &Joined,
    origin: &str,
) -> Vec<DirectMessage> {
    conversations
        .iter()
        .filter_map(|dm| {
            let user_id = dm.get("userId").and_then(Value::as_i64)?;
            let last_message_at = dm.get("lastMessageAt").and_then(as_time);

            if let Some(closed_at) = dm.get("closedAt").and_then(as_time) {
                if last_message_at.is_none_or(|at| at <= closed_at) {
                    return None;
                }
            }

            Some(DirectMessage {
                avatar_url: joined
                    .user_avatars
                    .get(&user_id)
                    .and_then(|avatar| avatar.url(origin)),
                channel_id: dm.get("channelId").and_then(Value::as_i64)?,
                user_name: joined
                    .user_names
                    .get(&user_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown".into()),
                last_message_at,
            })
        })
        .collect()
}

/// The shared unread floor from the plugin's row, if it holds one.
pub(crate) fn parse_shared_floor(stored: &Value) -> Option<HashMap<i64, u32>> {
    stored.get("readFloor").and_then(channel_counts)
}

pub(crate) fn parse_message(data: &Value) -> Option<Event> {
    let content = data.get("content").and_then(Value::as_str).unwrap_or("");
    let mentions = mentioned(content);

    Some(Event::Posted(NewMessage {
        channel_id: data.get("channelId").and_then(Value::as_i64)?,
        user_id: data.get("userId").and_then(Value::as_i64),
        plugin_id: data
            .get("pluginId")
            .and_then(Value::as_str)
            .map(str::to_string),
        text: plain_text(content),
        mentioned: mentions.users,
        mentioned_roles: mentions.roles,
    }))
}

/// A read-state frame: `delta` for a new message, `count` for a read.
pub(crate) fn parse_delta(data: &Value) -> Option<Event> {
    let channel_id = data.get("channelId").and_then(Value::as_i64)?;

    if let Some(delta) = data.get("delta").and_then(as_integer) {
        return Some(Event::Unread { channel_id, delta });
    }

    Some(Event::UnreadSet {
        channel_id,
        count: data.get("count").and_then(as_count)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
                message: "You must be authenticated.".into(),
                busy: false
            }
        );
        assert_eq!(
            parse_reply(
                r#"{"id":3,"error":{"message":"Too many requests.","code":-32029,"data":{"code":"TOO_MANY_REQUESTS","httpStatus":429}}}"#
            ),
            Reply::Failed {
                id: Some(3),
                message: "Too many requests.".into(),
                busy: true
            }
        );
        assert_eq!(
            parse_reply(&format!(
                r#"{{"id":2,"error":{{"message":"see https://evil.example {}"}}}}"#,
                "x".repeat(300)
            )),
            Reply::Failed {
                id: Some(2),
                message: format!("see {}", "x".repeat(196)),
                busy: false
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
            "ownUserId": 2,
            "users": [{ "id": 2, "name": "Smiddy", "roleIds": [1, 4] }, { "id": 3, "name": "Other", "roleIds": [9] }],
            "pluginsMetadata": [{ "pluginId": "other", "version": "2.0.0" }, { "pluginId": "shiver", "version": "0.1.0" }]
        }));

        assert_eq!(joined.own_role_ids, vec![1, 4]);

        assert_eq!(joined.read_states.get(&140), Some(&2));
        assert_eq!(joined.dm_channels, vec![9]);
        assert_eq!(
            joined.user_names.get(&2).map(String::as_str),
            Some("Smiddy")
        );
        assert_eq!(joined.plugin_version.as_deref(), Some("0.1.0"));
        assert_eq!(
            plugin_version(
                &serde_json::json!({ "pluginsMetadata": [{ "pluginId": "shiver", "version": "1.0\u{202e}" }] }),
                SHIVER_PLUGIN_ID
            ),
            None
        );
        assert_eq!(parse_join(&serde_json::json!({})), Joined::default());
    }

    #[test]
    fn conversations_carry_their_name_picture_and_latest_message() {
        let joined = parse_join(&serde_json::json!({
            "users": [
                { "id": 2, "name": "Smiddy", "avatar": { "name": "a b.png", "_accessToken": "t0k", "_accessTokenExpiresAt": 99.0 } },
                { "id": 3, "name": "Plain", "avatar": { "name": "p.png" } },
                { "id": 4, "name": "Nobody", "avatar": null }
            ]
        }));
        let dms = parse_dms(
            serde_json::json!([
                { "channelId": 9, "userId": 2, "lastMessageAt": 1_757_000_000_000u64 },
                { "channelId": 10, "userId": 404, "lastMessageAt": 1.757e12 },
                { "channelId": 11 },
                { "channelId": 12, "userId": 3 },
                { "channelId": 13, "userId": 4 }
            ])
            .as_array()
            .unwrap(),
            &joined,
            "https://chat.example.com",
        );

        assert_eq!(dms.len(), 4);
        assert_eq!(
            dms[0].avatar_url.as_deref(),
            Some("https://chat.example.com/public/a%20b.png?accessToken=t0k&expires=99")
        );
        assert_eq!(dms[1].avatar_url, None, "unknown user");
        assert_eq!(
            dms[2].avatar_url.as_deref(),
            Some("https://chat.example.com/public/p.png")
        );
        assert_eq!(dms[3].avatar_url, None, "no picture");
        assert_eq!(
            (dms[0].user_name.as_str(), dms[0].last_message_at),
            ("Smiddy", Some(1_757_000_000_000))
        );
        assert_eq!(
            (dms[1].user_name.as_str(), dms[1].last_message_at),
            ("Unknown", Some(1_757_000_000_000))
        );
    }

    /// Sharkord 0.0.28 lists the conversations in the join, each with when the user closed it.
    #[test]
    fn the_join_lists_conversations_and_closed_ones_stay_closed_until_a_message() {
        let data = serde_json::json!({
            "users": [{ "id": 2, "name": "Smiddy" }],
            "directMessages": [
                { "channelId": 9, "userId": 2, "lastMessageAt": 200, "closedAt": null },
                { "channelId": 10, "userId": 2, "lastMessageAt": 200, "closedAt": 300 },
                { "channelId": 11, "userId": 2, "lastMessageAt": 400, "closedAt": 300 },
                { "channelId": 12, "userId": 2, "closedAt": 300 }
            ]
        });
        let joined = parse_join(&data);
        let listed = join_dms(&data).unwrap();
        let dms = parse_dms(listed, &joined, "https://chat.example.com");

        assert_eq!(
            dms.iter().map(|dm| dm.channel_id).collect::<Vec<_>>(),
            [9, 11]
        );
        assert!(
            join_dms(&serde_json::json!({ "users": [] })).is_none(),
            "an older server"
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
                text: "shape check".into(),
                mentioned: Vec::new(),
                mentioned_roles: Vec::new(),
            }))
        );
        assert_eq!(parse_message(&serde_json::json!({ "content": "hi" })), None);

        let Some(Event::Posted(message)) = parse_message(&serde_json::json!({
            "content": r#"<p><span data-type="role-mention" data-role-id="3">@Mods</span></p>"#,
            "channelId": 8
        })) else {
            panic!("a message");
        };

        assert_eq!(message.mentioned_roles, [3]);
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
}
