//! UnifiedPush: one registration per chosen server, so the companion plugin can wake a closed phone.
//!
//! Each rail entry registers under its own random `push_token` (never the entry id, which pages see),
//! because the Android receivers act on any broadcast carrying a known token. Endpoints arrive
//! asynchronously as broadcasts and are handed to the server's page to register with the plugin.

use std::{collections::HashMap, sync::Mutex};

use shiver_core::LockExt;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shiver_push::{PushEvent, PushExt};

use crate::{
    error::{Error, Result},
    inbox,
    store::{RegistryStore, Store},
};

/// Tells Shiver's pages that registration state changed.
pub const PUSH_EVENT: &str = "shiver://push";

#[derive(Default)]
struct State {
    /// entry id -> endpoint
    endpoints: HashMap<String, String>,
    failed: Vec<String>,
}

#[derive(Default)]
pub struct Push(Mutex<State>);

impl Push {
    pub fn endpoint(&self, entry_id: &str) -> Option<String> {
        self.0.locked().endpoints.get(entry_id).cloned()
    }

    pub fn has_failed(&self, entry_id: &str) -> bool {
        self.0.locked().failed.iter().any(|id| id == entry_id)
    }

    /// (registered, failed)
    pub fn snapshot(&self) -> (usize, usize) {
        let state = self.0.locked();

        (state.endpoints.len(), state.failed.len())
    }

    fn update(&self, entry_id: &str, endpoint: Option<String>, failed: bool) {
        let mut state = self.0.locked();

        state.failed.retain(|id| id != entry_id);

        match endpoint {
            Some(endpoint) => {
                state.endpoints.insert(entry_id.to_string(), endpoint);
            }
            None => {
                state.endpoints.remove(entry_id);
            }
        }

        if failed {
            state.failed.push(entry_id.to_string());
        }
    }
}

fn entry_for(app: &AppHandle, token: &str) -> Option<String> {
    app.state::<Store>()
        .registry()
        .entry_for_push_token(token)
        .map(|entry| entry.id.clone())
}

/// Gives every entry a push token, and unregisters any old registration made under an entry id.
pub fn migrate_tokens(app: &AppHandle) {
    let migrated = app
        .state::<Store>()
        .update(|registry| Ok(registry.ensure_push_tokens()))
        .unwrap_or_default();

    let wanted = app
        .state::<Store>()
        .registry()
        .settings
        .push_servers
        .clone();

    for entry_id in migrated.iter().filter(|id| wanted.contains(id)) {
        let _ = app.shiver_push().unregister(entry_id);
    }
}

/// Listens for distributor broadcasts. Registered before `register_wanted`, whose answers are those
/// broadcasts.
pub fn start(app: &AppHandle) {
    let handle = app.clone();

    let result = app.shiver_push().on_event(move |event| {
        let (token, endpoint, failed) = match &event {
            PushEvent::Endpoint { token, endpoint } => (token, Some(endpoint.clone()), false),
            PushEvent::Failed { token } => (token, None, true),
            PushEvent::Unregistered { token } => (token, None, false),
            PushEvent::Message { token } => {
                // a running app turns a wake-up into a sync rather than a notification
                if entry_for(&handle, token).is_some() {
                    inbox::sync(&handle);
                }

                return;
            }
        };

        if let Some(entry_id) = entry_for(&handle, token) {
            handle.state::<Push>().update(&entry_id, endpoint, failed);
            let _ = handle.emit(PUSH_EVENT, ());
        }
    });

    if let Err(error) = result {
        eprintln!("[shiver] push events are unavailable: {error}");
    }
}

/// Asks the distributor for an endpoint for every chosen server.
pub fn register_wanted(app: &AppHandle) {
    let servers: Vec<(String, String)> = {
        let store = app.state::<Store>();
        let registry = store.registry();

        registry
            .servers
            .iter()
            .filter(|server| registry.settings.push_servers.contains(&server.id))
            .filter_map(|server| Some((server.push_token.clone()?, server.name.clone())))
            .collect()
    };

    for (token, name) in servers {
        if let Err(error) = app.shiver_push().register(&token, &name) {
            eprintln!("[shiver] could not register {name} for push: {error}");
        }
    }
}

/// Turns one server's waking on or off.
pub fn set_wanted(app: &AppHandle, entry_id: &str, wanted: bool) -> Result<()> {
    let retired = app.state::<Push>().endpoint(entry_id).filter(|_| !wanted);
    let target = app.state::<Store>().update(|registry| {
        registry.settings.push_servers.retain(|id| id != entry_id);

        if wanted {
            registry.settings.push_servers.push(entry_id.to_string());
        }

        let Some(server) = registry.server_mut(entry_id) else {
            return Ok(None);
        };

        if let Some(endpoint) =
            retired.filter(|endpoint| !server.retired_push_endpoints.contains(endpoint))
        {
            server.retired_push_endpoints.push(endpoint);
        }

        Ok(server
            .push_token
            .clone()
            .map(|token| (token, server.name.clone())))
    })?;

    let Some((token, name)) = target else {
        return Ok(());
    };

    let result = if wanted {
        app.shiver_push().register(&token, &name)
    } else {
        app.state::<Push>().update(entry_id, None, false);
        app.shiver_push().unregister(&token)
    };

    result.map_err(|error| Error::Webview(error.to_string()))
}

/// Unregisters a server that is leaving the rail. Call before it is removed from the registry.
pub fn unregister(app: &AppHandle, entry_id: &str) {
    app.state::<Push>().update(entry_id, None, false);

    let token = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .and_then(|server| server.push_token.clone());

    if let Some(token) = token {
        if let Err(error) = app.shiver_push().unregister(&token) {
            eprintln!("[shiver] could not unregister {entry_id} from push: {error}");
        }
    }
}
