//! Session tokens for new pages and sockets, and signing back in when a page reports that its
//! seeded session was refused.
//!
//! A token due for renewal is used as it is and renewed in the background, so opening a page only
//! waits on a sign-in when there is no usable session at all.
//!
//! Recovery attempts are capped and spaced, and the count resets only when a page actually reports
//! it is signed in again — so a page that keeps claiming "signed out" (a hostile one, or a Sharkord
//! change Shiver does not understand) cannot make Shiver re-send the password forever.
//!
//! Choosing Disconnect on the server's own page is not a refused session: Shiver then stays signed
//! out of that entry, across restarts, until the user signs in again on the page or with Shiver's
//! form. The password is kept for that (`disconnected_by_user`).

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use shiver_core::{
    jwt::{self, Freshness, Renewals},
    LockExt,
};
use tauri::{AppHandle, Manager};

use crate::{
    drain::Readiness,
    model::ServerEntry,
    secrets::{self, Secret},
    store::{RegistryStore, Store},
    voice::VoiceState,
    webviews::{self, ActiveServer},
};

/// Attempts allowed without a confirmed sign-in in between.
const MAX_ATTEMPTS: u32 = 3;

/// Minimum spacing between attempts.
const RETRY_AFTER: Duration = Duration::from_secs(20);

/// A session token for this entry.
///
/// Never fails: with no usable session the server's own login page is the fallback. A token due for
/// renewal is returned as it is and renewed in the background; only a missing or expired one waits
/// on a sign-in.
pub async fn token(app: &AppHandle, entry: &ServerEntry) -> Option<String> {
    session(app, entry).await.ok()
}

/// A session token, or why there is none: no way to sign in (no password, or the server refused it,
/// which only the user can answer), or the server could not be reached to sign in (which time does).
pub async fn session(app: &AppHandle, entry: &ServerEntry) -> Result<String, NoSession> {
    // asked of the registry, not `entry`: a socket's copy may be older than the user's Disconnect
    if is_disconnected(app, &entry.id) {
        return Err(NoSession::Signed);
    }

    let stored = secrets::read_off_thread(Secret::Session, &entry.id)
        .await
        .map(|token| token.to_string());

    match stored.as_deref().map(jwt::freshness) {
        Some(Freshness::Fresh) => stored.ok_or(NoSession::Signed),
        Some(Freshness::Due) => {
            renew_in_background(app, entry);

            stored.ok_or(NoSession::Signed)
        }
        Some(Freshness::Expired) | None => renew(app, entry).await,
    }
}

/// Why `session` has no token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoSession {
    /// no stored password, or the server refused it: only the user signing in will do
    Signed,
    /// the server did not answer, or not as Sharkord: worth asking again later
    Unreachable,
}

/// A session to seed a new page with: `None` when the entry's page already exists, since it has
/// its own (and a rail click should not touch the keychain).
pub async fn token_for_new_page(app: &AppHandle, entry: &ServerEntry) -> Option<String> {
    match app.get_webview(&webviews::webview_label(&entry.id)) {
        Some(_) => None,
        None => token(app, entry).await,
    }
}

fn renew_in_background(app: &AppHandle, entry: &ServerEntry) {
    if entry.identity.is_none() || !app.state::<Renewals>().begin(&entry.id) {
        return;
    }

    let (app, entry) = (app.clone(), entry.clone());

    tauri::async_runtime::spawn(async move {
        if renew(&app, &entry).await.is_ok() {
            app.state::<Renewals>().succeeded(&entry.id);
        }
    });
}

/// Held while a renewed session is checked and stored, and while a log-out or removal clears an
/// entry's identity and secrets, so a renewal that finishes during one cannot store a session after
/// it (which would sign the page back in the next time it is built).
pub static CREDENTIALS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Whether the entry is still listed and signed in through Shiver: not removed, logged out of, or
/// disconnected from on its page.
fn still_signed_in(app: &AppHandle, entry_id: &str) -> bool {
    app.state::<Store>()
        .registry()
        .server(entry_id)
        .is_some_and(|entry| entry.identity.is_some() && !entry.disconnected)
}

/// Whether the user chose Disconnect on this entry's page and has not signed in again since.
pub fn is_disconnected(app: &AppHandle, entry_id: &str) -> bool {
    app.state::<Store>()
        .registry()
        .server(entry_id)
        .is_some_and(|entry| entry.disconnected)
}

/// The user chose Disconnect on the server's own page. As in a browser, the session ends and the
/// page waits at its sign-in. Unlike a refused session, Shiver does not sign back in: not now, not
/// after a restart, and not for a new page or a socket, until the user signs in again. The password
/// stays, so that takes one click in Shiver's form.
pub fn disconnected_by_user(app: &AppHandle, entry_id: &str) {
    let entry = {
        let store = app.state::<Store>();
        let registry = store.registry();

        // only an entry Shiver signs in has anything to stop
        registry
            .server(entry_id)
            .filter(|entry| entry.identity.is_some() && !entry.disconnected)
            .cloned()
    };

    let Some(entry) = entry else {
        return;
    };

    let marked = app.state::<Store>().update(|registry| {
        if let Some(server) = registry.server_mut(entry_id) {
            server.disconnected = true;
        }

        Ok(())
    });

    if let Err(error) = marked {
        log::warn!("could not keep {} disconnected: {error}", entry.origin);

        return;
    }

    app.state::<Recovery>().forget_entry(entry_id);

    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        {
            // after any renewal that is storing a session right now, which this then takes back
            let _held = CREDENTIALS.lock().await;

            if let Err(error) = secrets::forget_off_thread(Secret::Session, &entry.id).await {
                log::warn!("could not drop the session for {}: {error}", entry.origin);
            }
        }

        // the page keeps the session it was built with, and would seed it again on a reload
        if app
            .get_webview(&webviews::webview_label(&entry.id))
            .is_some()
        {
            rebuild_pages(&app, &entry, None);
        }
    });
}

/// The page reports it is signed in. After a Disconnect that is the user signing in again on the
/// page, so Shiver goes back to signing this entry in itself.
pub fn signed_in_on_page(app: &AppHandle, entry_id: &str) {
    if !is_disconnected(app, entry_id) {
        return;
    }

    let cleared = app.state::<Store>().update(|registry| {
        if let Some(server) = registry.server_mut(entry_id) {
            server.disconnected = false;
        }

        Ok(())
    });

    match cleared {
        // a socket that found no session was parked
        Ok(()) => crate::watch::restart(app, entry_id),
        Err(error) => log::warn!("could not record signing in to {entry_id} again: {error}"),
    }
}

/// Signs in with the stored password and keeps the new session, unless the entry was logged out of,
/// disconnected from or removed while it signed in.
async fn renew(app: &AppHandle, entry: &ServerEntry) -> Result<String, NoSession> {
    // the password is not even sent while the user stays disconnected
    if is_disconnected(app, &entry.id) {
        return Err(NoSession::Signed);
    }

    let (Some(identity), Some(password)) = (
        entry.identity.as_deref(),
        secrets::read_off_thread(Secret::Password, &entry.id).await,
    ) else {
        return Err(NoSession::Signed);
    };

    match shiver_core::login::sign_in(&entry.origin, identity, &password).await {
        Ok(token) => {
            let _held = CREDENTIALS.lock().await;

            if !still_signed_in(app, &entry.id) {
                return Err(NoSession::Signed);
            }

            if let Err(error) = secrets::store_off_thread(Secret::Session, &entry.id, &token).await
            {
                log::warn!(
                    "could not keep the new session for {}: {error}",
                    entry.origin
                );
            }

            Ok(token)
        }
        Err(error) => {
            log::warn!("could not renew the session for {}: {error}", entry.origin);

            Err(match error {
                shiver_core::Error::Unreachable(_) | shiver_core::Error::NotSharkord(_) => {
                    NoSession::Unreachable
                }
                _ => NoSession::Signed,
            })
        }
    }
}

#[derive(Default)]
struct Attempt {
    /// a sign-in is running, so the page stays covered
    in_flight: bool,
    /// attempts since the last confirmed sign-in
    attempts: u32,
    last: Option<Instant>,
    /// the user has already been asked for credentials once
    prompted: bool,
}

#[derive(Default)]
pub struct Recovery(Mutex<HashMap<String, Attempt>>);

impl Recovery {
    pub fn in_flight(&self, entry_id: &str) -> bool {
        self.0
            .locked()
            .get(entry_id)
            .is_some_and(|attempt| attempt.in_flight)
    }

    /// Claims the right to sign this entry in again, or declines.
    fn begin(&self, entry_id: &str) -> bool {
        let mut attempts = self.0.locked();
        let attempt = attempts.entry(entry_id.to_string()).or_default();

        if attempt.in_flight
            || attempt.attempts >= MAX_ATTEMPTS
            || attempt
                .last
                .is_some_and(|last| last.elapsed() < RETRY_AFTER)
        {
            return false;
        }

        attempt.in_flight = true;
        attempt.attempts += 1;
        attempt.last = Some(Instant::now());

        true
    }

    fn finish(&self, entry_id: &str) {
        if let Some(attempt) = self.0.locked().get_mut(entry_id) {
            attempt.in_flight = false;
        }
    }

    /// The page reported itself signed in: the history is cleared.
    pub fn confirmed(&self, entry_id: &str) {
        if let Some(attempt) = self.0.locked().get_mut(entry_id) {
            if !attempt.in_flight {
                *attempt = Attempt::default();
            }
        }
    }

    /// Claims the single chance to ask the user for this server's credentials.
    pub fn take_prompt(&self, entry_id: &str) -> bool {
        let mut attempts = self.0.locked();
        let attempt = attempts.entry(entry_id.to_string()).or_default();

        !std::mem::replace(&mut attempt.prompted, true)
    }

    pub fn forget_entry(&self, entry_id: &str) {
        self.0.locked().remove(entry_id);
    }
}

/// Starts signing an entry back in with its stored password. Returns whether Shiver is handling it
/// (which keeps the page covered); `false` means the login form is the user's to answer.
pub fn recover(app: &AppHandle, entry_id: &str) -> bool {
    let Some(entry) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .filter(|entry| entry.identity.is_some() && !entry.disconnected)
        .cloned()
    else {
        return false;
    };

    let recovery = app.state::<Recovery>();

    if !recovery.begin(entry_id) {
        return recovery.in_flight(entry_id);
    }

    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        if !sign_in_again(&app, &entry).await {
            log::warn!(
                "could not sign back in to {}; its login page is the user's",
                entry.origin
            );
        }

        app.state::<Recovery>().finish(&entry.id);
    });

    true
}

async fn sign_in_again(app: &AppHandle, entry: &ServerEntry) -> bool {
    let Ok(token) = renew(app, entry).await else {
        return false;
    };

    rebuild_pages(app, entry, Some(&token));

    true
}

/// Rebuilds an entry's pages so the bridge seeds the new session (or, with none, none at all),
/// leaving the user where they were.
fn rebuild_pages(app: &AppHandle, entry: &ServerEntry, token: Option<&str>) {
    let (settings, muted) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        (registry.settings.clone(), registry.muted_for(&entry.id))
    };

    let (was_showing, conversation) = {
        let active = app.state::<ActiveServer>();

        (
            active.get().as_deref() == Some(entry.id.as_str()) && active.showing_server(),
            // a conversation beside Shiver's DM list is on screen too, and closing forgets it
            active.conversation_with(&entry.id),
        )
    };

    if let Err(error) = webviews::close_server(app, &entry.id) {
        log::warn!("could not close {} to rebuild it: {error}", entry.origin);

        return;
    }

    app.state::<Readiness>().forget_entry(&entry.id);

    let locked = app
        .state::<VoiceState>()
        .holder()
        .is_some_and(|holder| holder != entry.id);

    let built = if let Some(name) = conversation {
        webviews::show_conversation(app, entry, &settings, token, &muted, locked, &name)
    } else if was_showing {
        webviews::show_server(app, entry, &settings, token, &muted, locked)
    } else {
        webviews::preload_server(app, entry, &settings, token, &muted, locked)
    };

    if let Err(error) = built {
        log::warn!("could not rebuild {}: {error}", entry.origin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear_cooldown(recovery: &Recovery, entry_id: &str) {
        if let Some(attempt) = recovery.0.locked().get_mut(entry_id) {
            attempt.last = None;
        }
    }

    #[test]
    fn one_attempt_at_a_time_and_not_immediately_again() {
        let recovery = Recovery::default();

        assert!(recovery.begin("a"));
        assert!(recovery.in_flight("a"));
        assert!(!recovery.begin("a"));

        recovery.finish("a");
        assert!(!recovery.begin("a"), "cooldown");
    }

    /// Successful sign-ins that never lead to a signed-in page count too: that is the loop.
    #[test]
    fn attempts_stop_until_a_page_confirms_a_sign_in() {
        let recovery = Recovery::default();

        for _ in 0..MAX_ATTEMPTS {
            clear_cooldown(&recovery, "a");
            assert!(recovery.begin("a"));
            recovery.finish("a");
        }

        clear_cooldown(&recovery, "a");
        assert!(!recovery.begin("a"));
        assert!(recovery.begin("b"), "per entry");

        recovery.confirmed("a");
        assert!(recovery.begin("a"));
    }

    #[test]
    fn the_user_is_prompted_once() {
        let recovery = Recovery::default();

        assert!(recovery.take_prompt("a"));
        assert!(!recovery.take_prompt("a"));

        recovery.forget_entry("a");
        assert!(recovery.take_prompt("a"));
    }
}
