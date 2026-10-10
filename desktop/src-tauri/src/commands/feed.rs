//! The bell's notifications, the DM list, unread counts and channel mutes.

use tauri::{AppHandle, Manager, State};

use crate::{
    drain,
    error::Result,
    feed::{DmEntry, Feed, FeedSummary, Notification},
    store::{RegistryStore, Store},
    webviews,
};

#[tauri::command]
pub fn list_notifications(feed: State<'_, Feed>) -> Vec<Notification> {
    feed.notifications()
}

#[tauri::command]
pub fn list_dms(feed: State<'_, Feed>) -> Vec<DmEntry> {
    feed.dms()
}

/// The bell's count (the feed only, since every entry there has a message to show) and newest entry.
#[tauri::command]
pub fn feed_summary(feed: State<'_, Feed>) -> FeedSummary {
    feed.summary()
}

/// Per-entry rail badges: the feed's unread plus what arrived while Shiver was closed.
#[tauri::command]
pub fn unread_counts(
    app: AppHandle,
    feed: State<'_, Feed>,
) -> std::collections::HashMap<String, usize> {
    let mut counts = feed.unread_by_entry();

    for (entry_id, missed) in app.state::<crate::watch::Missed>().counts() {
        *counts.entry(entry_id).or_default() += missed;
    }

    counts
}

/// "Mark all as read": Shiver's notifications, the missed count, and Sharkord's own channel state.
#[tauri::command]
pub async fn mark_server_read(app: AppHandle, entry_id: String) -> Result<()> {
    webviews::mark_all_read(&app, &entry_id);
    crate::watch::mark_read(&app, &entry_id);

    if app.state::<Feed>().mark_entry_read(&entry_id) {
        drain::notify_feed_changed(&app);
    }

    Ok(())
}

#[tauri::command]
pub fn mark_notifications_read(app: AppHandle, feed: State<'_, Feed>) {
    feed.mark_all_read();
    drain::notify_feed_changed(&app);
}

#[tauri::command]
pub fn clear_notifications(app: AppHandle, feed: State<'_, Feed>) {
    feed.clear();
    drain::notify_feed_changed(&app);
}

#[tauri::command]
pub async fn set_channel_muted(
    app: AppHandle,
    store: State<'_, Store>,
    entry_id: String,
    channel_id: i64,
    muted: bool,
) -> Result<()> {
    let remaining = store.update(|registry| {
        let mut channels = registry.muted_for(&entry_id);

        channels.retain(|channel| *channel != channel_id);

        if muted {
            channels.push(channel_id);
        }

        Ok(registry.set_muted_for(&entry_id, channels))
    })?;

    webviews::push_muted(&app, &entry_id, &remaining);
    crate::watch::mutes_changed(&app, &entry_id);

    Ok(())
}
