//! Where "the user is looking at this channel" brings badges down. The drain calls only
//! `channel_viewed`, so the wiring lives in one place.

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

/// Whether this entry's page is the thing on screen (not merely selected behind a panel).
pub fn is_on_screen<R: Runtime>(app: &AppHandle<R>, entry_id: &str) -> bool {
    app.state::<ActiveServer>().is_on_screen(entry_id)
}
