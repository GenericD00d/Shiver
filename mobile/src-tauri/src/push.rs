//! Being told there are messages while Shiver is not running.
//!
//! `inbox.rs` notifies from Shiver's own sockets, which live exactly as long as the process does.
//! Android kills that process, and from then on the phone is silent — the one gap the mobile client
//! has never closed. The usual answer is Google FCM, which would make a self-hosted client depend on
//! Google Play Services, so Shiver uses **UnifiedPush** instead: a distributor app the user already
//! runs holds one connection for every app on the phone, and hands each one an endpoint URL that its
//! server can post to.
//!
//! This module is the small part in the middle. The distributor and the broadcasts are
//! `tauri-plugin-shiver-push`; deciding who to wake is the Shiver plugin on the server. What happens
//! here is:
//!
//! - **Register one instance per rail entry**, so each server gets its own endpoint. The endpoint
//!   that receives a push is then the answer to "which server?", and the push itself can be empty.
//! - **Keep the endpoints in memory only.** The distributor reissues them on registration, so there
//!   is nothing worth persisting and one less place a capability can leak from.
//! - **Turn a push into a connection.** A wake-up says only "look again", so the answer is
//!   `inbox::sync`, which starts a socket for any server that has a session and is not already
//!   being watched. Everything after that is the notification path Shiver already had.
//!
//! The endpoint reaches the server through the Shiver plugin, not from here: the bridge is handed this
//! entry's own endpoint and hands it to the plugin's relay, which checks it server-side before
//! storing. See `plugin/server/push.js`.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shiver_push::{PushEvent, PushExt};

use crate::{inbox, store::Store};

/// Tells Shiver's own pages that registration moved, so the settings screen can redraw.
pub const PUSH_EVENT: &str = "shiver://push";

#[derive(Default)]
struct State {
    /// entry id -> the endpoint the distributor issued for it
    endpoints: HashMap<String, String>,
    /// entries the distributor refused, so the screen can say so rather than showing a blank
    failed: Vec<String>,
}

#[derive(Default)]
pub struct Push(Mutex<State>);

impl Push {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// This entry's own endpoint, and only ever this one.
    ///
    /// Handed to a server's page by the bridge payload, so the rule that governs everything else
    /// there governs this too: a page learns its own, never another's. Which is also why Shiver
    /// registers per entry rather than sharing one endpoint across servers.
    pub fn endpoint(&self, entry_id: &str) -> Option<String> {
        self.state().endpoints.get(entry_id).cloned()
    }

    /// Whether the distributor refused this one, so a row can say so rather than "waiting" forever.
    pub fn has_failed(&self, entry_id: &str) -> bool {
        self.state().failed.iter().any(|id| id == entry_id)
    }

    pub fn snapshot(&self) -> (usize, usize) {
        let state = self.state();

        (state.endpoints.len(), state.failed.len())
    }

    fn remember(&self, entry_id: String, endpoint: String) {
        let mut state = self.state();

        state.failed.retain(|id| id != &entry_id);
        state.endpoints.insert(entry_id, endpoint);
    }

    fn forget(&self, entry_id: &str) {
        self.state().endpoints.remove(entry_id);
    }

    fn mark_failed(&self, entry_id: String) {
        let mut state = self.state();

        state.endpoints.remove(&entry_id);

        if !state.failed.contains(&entry_id) {
            state.failed.push(entry_id);
        }
    }
}

/// Starts listening for what the distributor has to say, once, at startup.
///
/// Registration is deliberately not done here: it needs a distributor to have been chosen, and on a
/// first run there is not one. `register_wanted` runs after the user picks, and again at startup
/// for anyone who already has.
pub fn start(app: &AppHandle) {
    let handle = app.clone();

    let result = app.shiver_push().on_event(move |event| match event {
        PushEvent::Endpoint { token, endpoint } => {
            handle.state::<Push>().remember(token, endpoint);
            let _ = handle.emit(PUSH_EVENT, ());
        }
        PushEvent::Message { token } => {
            // A wake-up carries nothing, on purpose. What it means is "there is something you have
            // not seen", and the way to find out what is the connection Shiver already knows how to
            // make — so this is the whole of the handling.
            eprintln!("[shiver] woken for {token}");
            inbox::sync(&handle);
        }
        PushEvent::Failed { token } => {
            handle.state::<Push>().mark_failed(token);
            let _ = handle.emit(PUSH_EVENT, ());
        }
        PushEvent::Unregistered { token } => {
            handle.state::<Push>().forget(&token);
            let _ = handle.emit(PUSH_EVENT, ());
        }
    });

    if let Err(error) = result {
        eprintln!("[shiver] push events are unavailable: {error}");
    }
}

/// Asks the distributor for an endpoint for every server in the rail.
///
/// Cheap to call more than once: registering an entry that already has an endpoint simply produces
/// the same one again. The server's display name goes with it, so the cold-start receiver can name
/// the server without a network call — see `PushReceiver` in the Android plugin.
pub fn register_wanted(app: &AppHandle) {
    let servers = {
        let store = app.state::<Store>();
        let registry = store.registry();
        let wanted = registry.settings.push_servers.clone();

        registry
            .servers
            .iter()
            .filter(|server| wanted.contains(&server.id))
            .map(|server| (server.id.clone(), server.name.clone()))
            .collect::<Vec<_>>()
    };

    for (entry_id, name) in servers {
        if let Err(error) = app.shiver_push().register(&entry_id, &name) {
            eprintln!("[shiver] could not register {name} for push: {error}");
        }
    }
}

/// Starts or stops one server waking the phone.
///
/// Registering asks the distributor for an endpoint, which arrives later as a broadcast — so this
/// returns before there is one, and the settings screen listens for the event rather than a result.
/// Unregistering tells the distributor to drop it and forgets the endpoint here; the server itself
/// finds out when its next push comes back `404` or `410`, which its half of the plugin prunes on.
pub fn set_wanted(app: &AppHandle, entry_id: &str, wanted: bool) -> crate::error::Result<()> {
    let store = app.state::<Store>();

    let name = store.update(|registry| {
        let chosen = &mut registry.settings.push_servers;

        chosen.retain(|id| id != entry_id);

        if wanted {
            chosen.push(entry_id.to_string());
        }

        Ok(registry
            .servers
            .iter()
            .find(|server| server.id == entry_id)
            .map(|server| server.name.clone()))
    })?;

    let Some(name) = name else {
        return Ok(());
    };

    let push = app.shiver_push();

    let result = if wanted {
        push.register(entry_id, &name)
    } else {
        app.state::<Push>().forget(entry_id);
        push.unregister(entry_id)
    };

    result.map_err(|error| crate::error::Error::Webview(error.to_string()))
}

/// Stops one server being woken, for a server leaving the rail.
pub fn unregister(app: &AppHandle, entry_id: &str) {
    app.state::<Push>().forget(entry_id);

    if let Err(error) = app.shiver_push().unregister(entry_id) {
        eprintln!("[shiver] could not unregister {entry_id} from push: {error}");
    }
}
