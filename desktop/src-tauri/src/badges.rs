//! Where "the user is looking at this channel" brings badges down. Both drain paths (the server
//! page and the conversation view) call only `channel_viewed`, so the wiring lives in one place.

use tauri::{AppHandle, Manager, Runtime};

use crate::{drain, feed::Feed, webviews::ActiveServer};

/// Marks the channel's notifications read (the bell) and clears it from the tracked read states
/// (the server icon). Sharkord marks a channel read as a side effect of showing it.
pub fn channel_viewed<R: Runtime>(app: &AppHandle<R>, entry_id: &str, channel_id: i64) {
    let feed_changed = app.state::<Feed>().mark_channel_read(entry_id, channel_id);

    crate::watch::channel_read(app, entry_id, channel_id);

    if feed_changed {
        drain::notify_feed_changed(app);
    }
}

/// Whether this entry's server page is the thing on screen (not merely selected behind a panel).
pub fn is_on_screen<R: Runtime>(app: &AppHandle<R>, entry_id: &str) -> bool {
    let active = app.state::<ActiveServer>();

    active.get().as_deref() == Some(entry_id) && active.showing_server()
}
