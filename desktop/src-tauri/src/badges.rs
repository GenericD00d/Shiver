//! One place where "the user is looking at this channel" becomes badges coming down.
//!
//! **This module exists because the same bug shipped four times.** Reading a direct message left
//! the server icon and the bell standing; it was reported, I fixed the half I could see, and it
//! went out unfixed again. Each time the piece I checked was right and the piece I did not check
//! was the one lying — a page reporting a hardcoded `null` channel, a count with two sources that
//! came down on different events, and an open server having no socket for anything to notice the
//! read through.
//!
//! None of those were arithmetic mistakes, so none could be caught by testing arithmetic. They were
//! *wiring* mistakes, and wiring spread over two call sites gets fixed in one of them. So both
//! drain paths — the server's own page, and the conversation view beside the inbox — call
//! [`channel_viewed`] and nothing else.
//!
//! The counting underneath is tested in `watch`, against the state maps directly, so "does reading
//! a channel actually bring the number down" is answered by a test rather than by a screenshot.

use tauri::{AppHandle, Manager, Runtime};

use crate::{drain, feed::Feed, webviews::ActiveServer};

/// The user is looking at this channel on this server, so it is read.
///
/// Both numbers come down, and they are separate numbers on purpose:
///
/// - the **bell** counts notifications Shiver collected, so that channel's entries are marked read;
/// - the **server icon** counts what the server itself calls unread, so that channel is cleared
///   from the tracked read states and the total worked out again.
///
/// Adding them together was the earlier mistake: it counted one message twice whenever a socket
/// reconnected, and the two halves answered different events — so reading something brought one
/// down and left the other standing, which is exactly how it was reported.
///
/// Clearing the channel is not a guess. Sharkord marks a channel read as a side effect of showing
/// it — `setSelectedChannelId` calls `markChannelAsRead` — so by the time a page reports viewing
/// one, the server already agrees.
pub fn channel_viewed<R: Runtime>(app: &AppHandle<R>, entry_id: &str, channel_id: i64) {
    let feed_changed = app.state::<Feed>().mark_channel_read(entry_id, channel_id);

    // recounts and notifies for itself when it changes anything
    crate::watch::channel_read(app, entry_id, channel_id);

    if feed_changed {
        drain::notify_feed_changed(app);
    }
}

/// Whether a page's report should be acted on, given what is actually on screen.
///
/// A hidden page still reports the channel it was left on, and Sharkord raises a notification for
/// the selected channel *precisely* when its window is hidden — so acting on every report would
/// quietly eat the badges for whatever channel the user left open somewhere else.
pub fn is_on_screen<R: Runtime>(app: &AppHandle<R>, entry_id: &str) -> bool {
    let active = app.state::<ActiveServer>();
    let on_screen = active.get().as_deref() == Some(entry_id) && active.showing_server();

    if !on_screen {
    }

    on_screen
}
