//! Direct-message lists: each server's conversations, from its last join or from its page while it
//! is on screen, ordered by the latest message heard from either.

use std::collections::HashMap;

use tauri::{AppHandle, Manager, Url};

use super::Inbox;
use crate::{sharkord, store::Store};

/// One conversation and the rail entry (account) it belongs to. Only Shiver's own pages see these.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmEntry {
    pub entry_id: String,
    pub server_name: String,
    pub account_label: Option<String>,
    pub channel_id: i64,
    pub user_name: String,
    pub last_message_at: Option<u64>,
    /// the other person's picture, a signed link on that server's own origin
    pub avatar_url: Option<String>,
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// Every server's conversations, each with the latest time known for it from any source (the list
/// orders them: `sortDms` in `shared/web/dms.ts`).
pub fn collect_dms(
    servers: &[crate::model::ServerEntry],
    dms: &HashMap<String, Vec<sharkord::DirectMessage>>,
    times: &HashMap<String, HashMap<i64, u64>>,
) -> Vec<DmEntry> {
    servers
        .iter()
        .flat_map(|server| {
            dms.get(&server.id)
                .into_iter()
                .flatten()
                .map(move |dm| DmEntry {
                    entry_id: server.id.clone(),
                    server_name: server.name.clone(),
                    account_label: server.account_label.clone(),
                    channel_id: dm.channel_id,
                    user_name: dm.user_name.clone(),
                    last_message_at: dm.last_message_at.max(
                        times
                            .get(&server.id)
                            .and_then(|times| times.get(&dm.channel_id))
                            .copied(),
                    ),
                    avatar_url: dm.avatar_url.clone(),
                })
        })
        .collect()
}

/// The most conversations kept per server, and taken from one read of the page.
pub(super) const MAX_DMS: usize = 500;

/// Messages the page on screen saw sent or received: they move its conversations up. A page's claim
/// about its own entry only, bounded, and never past now.
pub fn messages_seen(app: &AppHandle, entry_id: &str, seen: &[(i64, f64)]) {
    let inbox = app.state::<Inbox>();

    for &(channel_id, at) in seen.iter().take(MAX_DMS) {
        if at.is_finite() && at >= 0.0 {
            inbox.dm_active(entry_id, channel_id, at as u64);
        }
    }
}

/// One conversation as the page on screen lists it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageDm {
    pub channel_id: i64,
    pub name: String,
    #[serde(default)]
    pub icon_url: Option<String>,
}

const MAX_DM_NAME: usize = 100;

const MAX_URL: usize = 2048;

/// The page on screen's conversations, which replace what its last join said: the core has no
/// connection of its own to that server while its page is up (and none at all to one it has no
/// session for), so the page is the only one that knows.
pub fn page_dms(app: &AppHandle, entry_id: &str, dms: Vec<PageDm>) {
    let Some(origin) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .map(|server| server.origin.clone())
    else {
        return;
    };

    let listed = page_dm_list(&origin, dms);

    app.state::<Inbox>()
        .with(|state| state.dms.insert(entry_id.to_string(), listed));
}

/// A page's claim about its own server's conversations: bounded, names cleaned (none left empty),
/// pictures only from that server's own origin.
fn page_dm_list(origin: &str, dms: Vec<PageDm>) -> Vec<sharkord::DirectMessage> {
    dms.into_iter()
        .take(MAX_DMS)
        .filter_map(|dm| {
            let name = shiver_core::text::clamp(dm.name, MAX_DM_NAME);
            let own = |url: &String| {
                url.len() <= MAX_URL
                    && Url::parse(url).is_ok_and(|url| crate::model::is_same_origin(origin, &url))
            };

            (!name.trim().is_empty()).then(|| sharkord::DirectMessage {
                channel_id: dm.channel_id,
                user_name: name,
                last_message_at: None,
                avatar_url: dm.icon_url.filter(own),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, position: i32) -> crate::model::ServerEntry {
        crate::model::ServerEntry {
            id: id.into(),
            origin: format!("https://{id}.example.com"),
            name: id.to_uppercase(),
            position,
            ..Default::default()
        }
    }

    fn conversation(channel_id: i64, name: &str, at: Option<u64>) -> sharkord::DirectMessage {
        sharkord::DirectMessage {
            channel_id,
            user_name: name.into(),
            last_message_at: at,
            avatar_url: None,
        }
    }

    #[test]
    fn every_servers_conversations_are_collected() {
        let dms = HashMap::from([
            (
                "a".to_string(),
                vec![
                    conversation(1, "Robin", Some(3_000)),
                    conversation(2, "Wren", None),
                    conversation(4, "Ash", Some(2_000)),
                ],
            ),
            ("b".to_string(), vec![conversation(3, "Sam", Some(2_000))]),
        ]);

        let names = |servers: &[crate::model::ServerEntry]| {
            let mut names: Vec<_> = collect_dms(servers, &dms, &HashMap::new())
                .into_iter()
                .map(|dm| dm.user_name)
                .collect();

            names.sort();
            names
        };

        assert_eq!(
            names(&[entry("a", 0), entry("b", 1)]),
            vec!["Ash", "Robin", "Sam", "Wren"]
        );
        assert!(collect_dms(&[entry("c", 0)], &dms, &HashMap::new()).is_empty());

        // the latest time wins, whichever source gave it
        let times = HashMap::from([("a".to_string(), HashMap::from([(2, 9_000), (1, 1)]))]);
        let listed = collect_dms(&[entry("a", 0)], &dms, &times);
        let time_of = |name: &str| {
            listed
                .iter()
                .find(|dm| dm.user_name == name)
                .and_then(|dm| dm.last_message_at)
        };

        assert_eq!(time_of("Wren"), Some(9_000));
        assert_eq!(time_of("Robin"), Some(3_000));
    }

    #[test]
    fn a_pages_conversations_are_bounded_cleaned_and_pictured_only_from_its_server() {
        let dm = |channel_id, name: &str, icon: Option<&str>| PageDm {
            channel_id,
            name: name.into(),
            icon_url: icon.map(str::to_string),
        };
        let listed = page_dm_list(
            "https://a.example.com",
            vec![
                dm(1, "Robin", Some("https://a.example.com/public/r.png")),
                dm(2, "Wren\u{202e}", Some("https://evil.example.com/w.png")),
                dm(3, "\u{200b}", None),
                dm(4, &"x".repeat(500), Some("javascript:alert(1)")),
            ],
        );

        assert_eq!(
            listed
                .iter()
                .map(|dm| (dm.channel_id, dm.avatar_url.is_some()))
                .collect::<Vec<_>>(),
            vec![(1, true), (2, false), (4, false)]
        );
        assert_eq!(listed[1].user_name, "Wren");
        assert!(listed[2].user_name.chars().count() <= MAX_DM_NAME + 1);
        assert_eq!(
            page_dm_list(
                "https://a.example.com",
                (0..1_000).map(|id| dm(id, "n", None)).collect()
            )
            .len(),
            MAX_DMS
        );
    }
}
