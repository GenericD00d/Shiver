//! Signing back in when a page reports that its seeded session was refused.
//!
//! Attempts are capped and spaced, and the count resets only when a page actually reports it is
//! signed in again — so a page that keeps claiming "signed out" (a hostile one, or a Sharkord change
//! Shiver does not understand) cannot make Shiver re-send the password forever.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use shiver_core::LockExt;
use tauri::{AppHandle, Manager};

use crate::{
    drain::Readiness,
    model::ServerEntry,
    secrets::{self, Secret},
    store::Store,
    voice::VoiceState,
    webviews::{self, ActiveServer},
};

/// Attempts allowed without a confirmed sign-in in between.
const MAX_ATTEMPTS: u32 = 3;

/// Minimum spacing between attempts.
const RETRY_AFTER: Duration = Duration::from_secs(20);

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
        .filter(|entry| entry.identity.is_some())
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
            eprintln!(
                "[shiver] could not sign back in to {}; its login page is the user's",
                entry.origin
            );
        }

        app.state::<Recovery>().finish(&entry.id);
    });

    true
}

async fn sign_in_again(app: &AppHandle, entry: &ServerEntry) -> bool {
    let (Some(identity), Some(password)) = (
        entry.identity.as_deref(),
        secrets::read_off_thread(Secret::Password, &entry.id).await,
    ) else {
        return false;
    };

    let token = match shiver_core::login::sign_in(&entry.origin, identity, &password).await {
        Ok(token) => token,
        Err(error) => {
            eprintln!(
                "[shiver] signing back in to {} failed: {error}",
                entry.origin
            );

            return false;
        }
    };

    if let Err(error) = secrets::store_off_thread(Secret::Session, &entry.id, &token).await {
        eprintln!(
            "[shiver] could not keep the new session for {}: {error}",
            entry.origin
        );

        return false;
    }

    rebuild_pages(app, entry, &token);

    true
}

/// Rebuilds an entry's pages so the bridge seeds the new session, leaving the user where they were.
fn rebuild_pages(app: &AppHandle, entry: &ServerEntry, token: &str) {
    let (settings, muted) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        (registry.settings.clone(), registry.muted_for(&entry.id))
    };

    let was_showing = {
        let active = app.state::<ActiveServer>();

        active.get().as_deref() == Some(entry.id.as_str()) && active.showing_server()
    };

    if let Err(error) = webviews::close_server(app, &entry.id) {
        eprintln!(
            "[shiver] could not close {} to rebuild it: {error}",
            entry.origin
        );

        return;
    }

    app.state::<Readiness>().forget_entry(&entry.id);

    let locked = app
        .state::<VoiceState>()
        .holder()
        .is_some_and(|holder| holder != entry.id);

    let built = if was_showing {
        webviews::show_server(app, entry, &settings, Some(token), &muted, locked)
    } else {
        webviews::preload_server(app, entry, &settings, Some(token), &muted, locked)
    };

    if let Err(error) = built {
        eprintln!("[shiver] could not rebuild {}: {error}", entry.origin);
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
