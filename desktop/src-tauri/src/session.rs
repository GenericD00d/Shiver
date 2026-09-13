//! Signing back in, when a seeded session turns out to be dead.
//!
//! Shiver's promise is that the user never meets the web login page, and `ensure_session` keeps it by
//! replacing a token the *clock* says is nearly out. It cannot see the other way a session ends: a
//! token that still looks valid but the server refuses — invalidated, or issued by a server that
//! has since been restarted. The client then falls back to its login form, which is the one thing
//! Shiver exists to avoid, and nothing about the token's expiry would ever have predicted it.
//!
//! So the page is asked instead. When it reports that it has given up on the seeded session, Shiver
//! signs in again with the password in the keychain and rebuilds the page on the new token.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use tauri::{AppHandle, Manager};

use crate::{
    drain::Readiness,
    login,
    model::ServerEntry,
    secrets::{self, Secret},
    store::Store,
    voice::VoiceState,
    webviews::{self, ActiveServer},
};

/// Enough to ride out a server restart, few enough that a wrong password stops asking.
const MAX_FAILURES: u32 = 3;

/// Long enough that a server refusing sessions is not hammered for them.
const RETRY_AFTER: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Attempt {
    /// set while a sign-in is in flight, so Shiver keeps its loading circle up rather than uncovering
    /// the login form it is busy making unnecessary
    in_flight: bool,
    failures: u32,
    last: Option<Instant>,
    /// whether the user has already been asked for this server's credentials since Shiver ran out of
    /// ways to get them itself, so they are asked once rather than on every poll
    prompted: bool,
}

/// Per-entry history of signing back in.
///
/// Attempts are capped and spaced. A password that no longer works would otherwise mean signing in
/// on a loop forever, and stopping is not about sparing the server: after a few honest failures the
/// login form is the *right* thing for the user to see, because they are the only one who can fix
/// it from there.
#[derive(Default)]
pub struct Recovery(Mutex<HashMap<String, Attempt>>);

impl Recovery {
    fn attempts(&self) -> MutexGuard<'_, HashMap<String, Attempt>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether Shiver is mid-recovery for this entry, and should go on covering its page.
    pub fn in_flight(&self, entry_id: &str) -> bool {
        self.attempts()
            .get(entry_id)
            .is_some_and(|attempt| attempt.in_flight)
    }

    /// Claims the right to sign this entry back in, or declines.
    fn begin(&self, entry_id: &str) -> bool {
        let mut attempts = self.attempts();
        let attempt = attempts.entry(entry_id.to_string()).or_default();

        if attempt.in_flight || attempt.failures >= MAX_FAILURES {
            return false;
        }

        if attempt
            .last
            .is_some_and(|last| last.elapsed() < RETRY_AFTER)
        {
            return false;
        }

        attempt.in_flight = true;
        attempt.last = Some(Instant::now());

        true
    }

    fn finish(&self, entry_id: &str, recovered: bool) {
        let mut attempts = self.attempts();
        let attempt = attempts.entry(entry_id.to_string()).or_default();

        attempt.in_flight = false;
        attempt.failures = if recovered { 0 } else { attempt.failures + 1 };

        if recovered {
            attempt.prompted = false;
        }
    }

    /// Claims the one chance to ask the user for this server's credentials.
    ///
    /// Shiver asks when it has run out of ways to sign in on its own — no password to use, or a
    /// password the server keeps refusing. The page reports the login form on every poll, so
    /// without claiming it the dialog would be reopened every 750ms.
    pub fn take_prompt(&self, entry_id: &str) -> bool {
        let mut attempts = self.attempts();
        let attempt = attempts.entry(entry_id.to_string()).or_default();

        if attempt.prompted {
            return false;
        }

        attempt.prompted = true;

        true
    }

    /// Forgets an entry's history, so removing or signing into a server starts clean.
    pub fn forget_entry(&self, entry_id: &str) {
        self.attempts().remove(entry_id);
    }
}

/// Signs an entry back in and rebuilds its pages on the new session.
///
/// Returns whether Shiver is handling it, which is also what decides whether the page stays covered.
/// `false` means Shiver will not: there are no credentials to use, or it has tried and failed enough
/// times that the form belongs to the user now.
pub fn recover(app: &AppHandle, entry_id: &str) -> bool {
    let entry = {
        let store = app.state::<Store>();
        let registry = store.registry();

        registry
            .servers
            .iter()
            .find(|server| server.id == entry_id)
            .cloned()
    };

    // no stored username means the user chose to sign in on the server's own page, and that page is
    // now doing exactly what they asked of it
    let Some(entry) = entry.filter(|entry| entry.identity.is_some()) else {
        return false;
    };

    let recovery = app.state::<Recovery>();

    if !recovery.begin(entry_id) {
        // already running, or done trying
        return recovery.in_flight(entry_id);
    }

    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        let recovered = sign_in_again(&app, &entry).await;

        app.state::<Recovery>().finish(&entry.id, recovered);

        if !recovered {
            eprintln!(
                "[shiver] could not sign back in to {}, so its login page is the user's to answer",
                entry.origin
            );
        }
    });

    true
}

async fn sign_in_again(app: &AppHandle, entry: &ServerEntry) -> bool {
    let Some(identity) = entry.identity.as_deref() else {
        return false;
    };

    // Shiver signs a server back in with the password it was given when the server was added. Signing
    // in on the server's own page never reaches Shiver, so an entry can name a user it cannot
    // authenticate as — and the sign-in item in the rail's menu is how the user fixes that.
    let Ok(Some(password)) = secrets::read(Secret::Password, &entry.id) else {
        eprintln!(
            "[shiver] no stored password for {}, so it cannot be signed back in",
            entry.origin
        );

        return false;
    };

    let token = match login::sign_in(&entry.origin, identity, &password).await {
        Ok(token) => token,
        Err(error) => {
            eprintln!("[shiver] signing back in to {} failed: {error}", entry.origin);

            return false;
        }
    };

    if let Err(error) = secrets::store(Secret::Session, &entry.id, &token) {
        eprintln!(
            "[shiver] could not keep the new session for {}: {error}",
            entry.origin
        );

        return false;
    }

    eprintln!("[shiver] signed back in to {}", entry.origin);

    rebuild_pages(app, entry, &token);

    true
}

/// Rebuilds an entry's pages so the bridge seeds the new session.
///
/// A token reaches a page through its initialization script, which runs once, so a live page cannot
/// be handed a new one — it has to be built again. The user is left where they were: a server they
/// were looking at comes back up in front of them, one they were not stays hidden.
fn rebuild_pages(app: &AppHandle, entry: &ServerEntry, token: &str) {
    let (settings, muted) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let muted = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == entry.id)
            .map(|muted| muted.channel_id)
            .collect::<Vec<_>>();

        (registry.settings.clone(), muted)
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

    webviews::close_dm_view(app, &entry.id);

    // the page it is about to build has not connected yet, so Shiver waits on it as it would any
    // other: the loading circle goes back up instead of the stale login form staying on screen
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

        return;
    }

    if let Err(error) = webviews::preload_dm_view(app, entry, &settings, Some(token)) {
        eprintln!(
            "[shiver] could not rebuild the conversation view for {}: {error}",
            entry.origin
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clears the cooldown, so a test can reach the retry cap without waiting `RETRY_AFTER`.
    fn forget_when(recovery: &Recovery, entry_id: &str) {
        if let Some(attempt) = recovery.attempts().get_mut(entry_id) {
            attempt.last = None;
        }
    }

    #[test]
    fn a_refused_session_is_retried() {
        let recovery = Recovery::default();

        assert!(recovery.begin("a"));
        assert!(recovery.in_flight("a"));
    }

    /// The page reports the login form on every drain, so without this one refusal would start a
    /// sign-in every 750ms.
    #[test]
    fn only_one_sign_in_runs_at_a_time() {
        let recovery = Recovery::default();

        assert!(recovery.begin("a"));
        assert!(!recovery.begin("a"));
    }

    #[test]
    fn a_failure_is_not_retried_immediately() {
        let recovery = Recovery::default();

        recovery.begin("a");
        recovery.finish("a", false);

        // the cooldown is still running, and nothing is in flight to keep the page covered
        assert!(!recovery.begin("a"));
        assert!(!recovery.in_flight("a"));
    }

    /// After enough honest failures the login form is the right thing to show: the password no
    /// longer works, and the user is the only one who can fix that.
    #[test]
    fn a_password_that_keeps_failing_stops_being_retried() {
        let recovery = Recovery::default();

        for _ in 0..MAX_FAILURES {
            forget_when(&recovery, "a");

            assert!(recovery.begin("a"));

            recovery.finish("a", false);
        }

        forget_when(&recovery, "a");

        assert!(!recovery.begin("a"));
        assert!(!recovery.in_flight("a"));
    }

    #[test]
    fn a_recovered_session_forgets_its_failures() {
        let recovery = Recovery::default();

        for _ in 0..2 {
            forget_when(&recovery, "a");
            recovery.begin("a");
            recovery.finish("a", false);
        }

        forget_when(&recovery, "a");
        recovery.begin("a");
        recovery.finish("a", true);

        forget_when(&recovery, "a");

        assert!(recovery.begin("a"));
    }

    #[test]
    fn one_server_giving_up_does_not_stop_another() {
        let recovery = Recovery::default();

        for _ in 0..MAX_FAILURES {
            forget_when(&recovery, "a");
            recovery.begin("a");
            recovery.finish("a", false);
        }

        forget_when(&recovery, "a");

        assert!(!recovery.begin("a"));
        assert!(recovery.begin("b"));
    }

    /// Signing in by hand, or re-adding the server, is the user fixing exactly what Shiver gave up on.
    #[test]
    fn forgetting_an_entry_clears_its_history() {
        let recovery = Recovery::default();

        for _ in 0..MAX_FAILURES {
            forget_when(&recovery, "a");
            recovery.begin("a");
            recovery.finish("a", false);
        }

        recovery.forget_entry("a");

        assert!(recovery.begin("a"));
    }
}
