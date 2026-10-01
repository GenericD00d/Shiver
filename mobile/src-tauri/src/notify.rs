//! The Android notifications for what the core's connections hear: one per server, summarising a
//! burst, under the same id `PushReceiver` uses so either side can replace or clear the other's.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

use shiver_core::{text::clamp, LockExt};
use tauri::{AppHandle, Manager};

use crate::{inbox::Inbox, sharkord, store::Store, webview};

const MAX_AUTHOR: usize = 100;
const MAX_BODY: usize = 500;
const MAX_CHANNEL_NAME: usize = 100;

/// Messages arriving this close together are posted as one notification.
const NOTIFY_AFTER: Duration = Duration::from_millis(900);

#[derive(Debug, Clone, Default)]
struct Announcement {
    count: u32,
    latest: String,
}

#[derive(Default)]
struct State {
    announced: HashMap<String, Announcement>,
    /// entries with a notification post scheduled
    posting: HashSet<String>,
}

/// What each server's next notification will say.
#[derive(Default)]
pub struct Notices(Mutex<State>);

impl Notices {
    fn with<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        change(&mut self.0.locked())
    }
}

/// One notification id per server, shared with `PushReceiver` (which uses Java's
/// `String.hashCode` of the push token), so either side can replace or clear the other's.
fn notification_id(app: &AppHandle, entry_id: &str) -> i32 {
    let token = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .and_then(|server| server.push_token.clone())
        .unwrap_or_else(|| entry_id.to_string());

    shiver_core::hash::java_string(&token)
}

pub fn announce(
    app: &AppHandle,
    entry_id: &str,
    joined: &sharkord::Joined,
    message: &sharkord::NewMessage,
) {
    if app
        .state::<webview::Showing>()
        .kept_by_page(crate::inbox::WATCH_GRACE)
        .as_deref()
        == Some(entry_id)
    {
        return;
    }

    let (muted, server) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        (
            registry.muted_for(entry_id),
            registry
                .server(entry_id)
                .map(|server| (server.name.clone(), server.notify)),
        )
    };

    let Some((server, notify)) = server else {
        return;
    };

    if !notify.allows(
        joined.dm_channels.contains(&message.channel_id),
        message.mentions_me(joined),
    ) {
        return;
    }

    let Some(line) = notice(joined, &muted, message) else {
        return;
    };

    // one post per burst: the first message schedules it, later ones update what it will say
    let first = app.state::<Notices>().with(|state| {
        let announcement = state.announced.entry(entry_id.to_string()).or_default();

        announcement.count += 1;
        announcement.latest = line;

        state.posting.insert(entry_id.to_string())
    });

    if !first {
        return;
    }

    let (app, entry_id) = (app.clone(), entry_id.to_string());

    std::thread::spawn(move || {
        std::thread::sleep(NOTIFY_AFTER);

        let announcement = app.state::<Notices>().with(|state| {
            state.posting.remove(&entry_id);
            state.announced.get(&entry_id).cloned()
        });

        if let Some(announcement) = announcement {
            post(
                &app,
                notification_id(&app, &entry_id),
                server,
                &announcement,
            );
        }
    });
}

/// The notification line for a message, or `None` for the user's own or a muted channel's.
fn notice(
    joined: &sharkord::Joined,
    muted: &[i64],
    message: &sharkord::NewMessage,
) -> Option<String> {
    if message.is_own(joined) || muted.contains(&message.channel_id) {
        return None;
    }

    let author = clamp(message.author(joined), MAX_AUTHOR);
    let text = clamp(message.body().to_string(), MAX_BODY);
    let channel = (!joined.dm_channels.contains(&message.channel_id)).then(|| {
        clamp(
            joined
                .channel_names
                .get(&message.channel_id)
                .cloned()
                .unwrap_or_else(|| "a channel".into()),
            MAX_CHANNEL_NAME,
        )
    });

    Some(shiver_core::text::notice_line(
        &author,
        channel.as_deref(),
        &text,
    ))
}

fn post(app: &AppHandle, id: i32, title: String, announcement: &Announcement) {
    use tauri_plugin_notification::NotificationExt;

    let body = match announcement.count {
        0 | 1 => announcement.latest.clone(),
        count => format!("{}\n… and {} more", announcement.latest, count - 1),
    };

    if let Err(error) = app
        .notification()
        .builder()
        .id(id)
        .title(title)
        .body(body)
        .show()
    {
        eprintln!("[shiver] could not show a notification: {error}");
    }
}

/// Forgets what an entry's next notification would summarise (its session is gone).
pub fn forget(app: &AppHandle, entry_id: &str) {
    app.state::<Notices>()
        .with(|state| state.announced.remove(entry_id));
}

/// Removes an entry's notification, if one was posted.
pub fn clear(app: &AppHandle, entry_id: &str) {
    let announced = app.state::<Notices>().with(|state| {
        state.posting.remove(entry_id);
        state.announced.remove(entry_id).is_some()
    });

    #[cfg(mobile)]
    if announced {
        use tauri_plugin_notification::NotificationExt;

        let id = notification_id(app, entry_id);
        let app = app.clone();

        std::thread::spawn(move || {
            let _ = app.notification().remove_active(vec![id]);
        });
    }

    #[cfg(not(mobile))]
    let _ = announced;
}

/// Says once, with a notification, that a server sends more than Shiver accepts.
pub fn report_watch_problem(app: &AppHandle, entry_id: &str, size: usize) {
    let Some(name) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .map(|server| server.name.clone())
    else {
        return;
    };

    let size = sharkord::readable_size(size);

    let first = app.state::<Inbox>().remember_problem(
        entry_id,
        &format!("Sends {size} in one message, which is more than Shiver accepts — so nothing from this server reaches this phone."),
    );

    #[cfg(target_os = "android")]
    if first {
        use tauri_plugin_notification::NotificationExt;

        let _ = app
            .notification()
            .builder()
            .id(shiver_core::hash::java_string(&format!("problem:{entry_id}")))
            .title(format!("Shiver cannot watch {name}"))
            .body("This server sends more in one message than Shiver accepts, so its messages will not reach you here. Settings has the details.")
            .show();
    }

    #[cfg(not(target_os = "android"))]
    let _ = (first, name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> sharkord::Joined {
        sharkord::Joined {
            own_user_id: Some(1),
            dm_channels: vec![9],
            channel_names: HashMap::from([(4, "general".to_string())]),
            user_names: HashMap::from([(2, "Smiddy".to_string())]),
            ..Default::default()
        }
    }

    fn from(user_id: i64, channel_id: i64, text: &str) -> sharkord::NewMessage {
        sharkord::NewMessage {
            channel_id,
            user_id: Some(user_id),
            plugin_id: None,
            text: text.to_string(),
            mentioned: Vec::new(),
        }
    }

    #[test]
    fn a_message_reads_as_who_said_what_and_where() {
        assert_eq!(
            notice(&server(), &[], &from(2, 4, "hi?")),
            Some("Smiddy in #general: hi?".into())
        );
        assert_eq!(
            notice(&server(), &[], &from(2, 9, "hello")),
            Some("Smiddy: hello".into())
        );
        assert_eq!(
            notice(&server(), &[], &from(2, 4, "")),
            Some("Smiddy in #general: Sent an attachment".into())
        );
        assert_eq!(
            notice(&server(), &[], &from(77, 88, "hi")),
            Some("Someone in #a channel: hi".into())
        );
        assert_eq!(notice(&server(), &[], &from(1, 4, "mine")), None);
        assert_eq!(notice(&server(), &[4], &from(2, 4, "muted")), None);
    }

    #[test]
    fn long_fields_are_shortened_by_characters() {
        let long_name = sharkord::NewMessage {
            plugin_id: Some("é".repeat(5_000)),
            user_id: None,
            ..from(0, 4, "hello")
        };
        let line = notice(&server(), &[], &long_name).unwrap();

        assert!(line.starts_with(&"é".repeat(MAX_AUTHOR)));
        assert!(line.ends_with("in #general: hello"));

        let line = notice(&server(), &[], &from(2, 4, &"a".repeat(10_000))).unwrap();

        assert_eq!(
            line.chars().count(),
            "Smiddy in #general: ".len() + MAX_BODY + 1
        );
    }
}
