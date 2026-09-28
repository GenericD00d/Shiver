//! System notifications for what reaches the feed while Shiver's window is not in front, where the
//! bell and the taskbar badge are easy to miss. A DM also asks for attention (the taskbar flashes).
//!
//! The notification plugin runs without its page script (`RustOnly`), so server pages keep their
//! own `Notification`, which the bridge replaces. Toasts are posted silent: Shiver plays its own
//! ping (for what gets past the mutes).

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use shiver_core::{text::notice_line, LockExt};
use tauri::{AppHandle, Manager, UserAttentionType};
use tauri_plugin_notification::NotificationExt;

use crate::{feed::Notification, store::Store, webviews};

/// A busy channel posts at most one system notification per server this often; DMs always do.
const CHANNEL_SPACING: Duration = Duration::from_secs(5);

/// When each server last posted a system notification for a channel message.
#[derive(Default)]
pub struct Posted(Mutex<HashMap<String, Instant>>);

impl Posted {
    /// Claims the right to post for `entry_id` now, unless it posted less than `spacing` ago.
    fn claim(&self, entry_id: &str, spacing: Duration) -> bool {
        let mut posted = self.0.locked();

        if posted
            .get(entry_id)
            .is_some_and(|at| at.elapsed() < spacing)
        {
            return false;
        }

        posted.insert(entry_id.to_string(), Instant::now());

        true
    }

    pub fn forget_entry(&self, entry_id: &str) {
        self.0.locked().remove(entry_id);
    }
}

/// Tells the system about a notification that just reached the feed, if the window is not in
/// front: a toast (when the user wants them) and, for a DM, a request for attention.
pub fn announce(app: &AppHandle, notification: &Notification) {
    if webviews::is_in_front(app) {
        return;
    }

    let Ok(window) = webviews::main_window(app) else {
        return;
    };

    if notification.is_dm {
        let _ = window.request_user_attention(Some(UserAttentionType::Informational));
    }

    let wanted = app
        .state::<Store>()
        .registry()
        .settings
        .desktop_notifications;
    let spacing = if notification.is_dm {
        Duration::ZERO
    } else {
        CHANNEL_SPACING
    };

    if !wanted || !app.state::<Posted>().claim(&notification.entry_id, spacing) {
        return;
    }

    let channel = (!notification.is_dm)
        .then_some(notification.channel_name.as_deref())
        .flatten();

    if let Err(error) = app
        .notification()
        .builder()
        .title(&notification.server_name)
        .body(notice_line(
            &notification.author,
            channel,
            &notification.body,
        ))
        .show()
    {
        eprintln!("[shiver] could not show a system notification: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_notifications_are_spaced_per_server() {
        let posted = Posted::default();

        assert!(posted.claim("a", CHANNEL_SPACING));
        assert!(!posted.claim("a", CHANNEL_SPACING));
        assert!(posted.claim("b", CHANNEL_SPACING), "per server");
        assert!(posted.claim("a", Duration::ZERO), "a DM is never held back");

        posted.forget_entry("a");
        assert!(posted.claim("a", CHANNEL_SPACING));
    }
}
