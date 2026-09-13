//! UnifiedPush wake-ups, so Shiver can be told there are messages while it is not running.
//!
//! Shiver's mobile notifications come from its own sockets (`inbox.rs`), which run only while the
//! process does. Android kills that process, and from then on the phone is silent. The usual answer
//! is Google FCM, which would make a self-hosted client depend on Google Play Services — so Shiver
//! uses **UnifiedPush**: a distributor app the user already runs (ntfy, say) holds one connection
//! for every app on the phone and hands each one an endpoint URL its server can post to.
//!
//! The shape, end to end:
//!
//! 1. The core registers one UnifiedPush *instance per rail entry*, so every server gets its own
//!    endpoint. The endpoint that receives a push is therefore the answer to "which server?", and
//!    nothing has to travel in the payload.
//! 2. The distributor answers asynchronously with a `NEW_ENDPOINT` broadcast, which arrives here as
//!    an [`PushEvent::Endpoint`].
//! 3. The core hands that endpoint to the server through the Shiver plugin's relay, which checks it
//!    and stores it against that user.
//! 4. When a message arrives for a user who is away, the plugin posts an **empty body** to the
//!    endpoint. Empty because the relay sits outside the server's origin: anything in the payload is
//!    something a third party would get to read.
//! 5. Shiver wakes and finds out what actually happened over its own authenticated connection.
//!
//! Nothing here is exposed to a webview. The plugin declares no commands: an endpoint is a
//! capability to make someone's phone light up, and a server's page must never be able to ask for
//! one.

use serde::Deserialize;

#[cfg(target_os = "android")]
use tauri::plugin::PluginHandle;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.shiver.push";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Plugin(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What the distributor told us, once the Kotlin side has made sense of it.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PushEvent {
    /// an endpoint for one rail entry, which is what the server needs to be given
    Endpoint { token: String, endpoint: String },
    /// something arrived for this entry. Carries nothing else, by design.
    Message { token: String },
    /// the distributor refused, usually because it has not been set up yet
    Failed { token: String },
    /// this entry is no longer registered, so the server should stop being told to use it
    Unregistered { token: String },
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct DistributorsResponse {
    distributors: Vec<String>,
    /// the one already chosen, or empty
    saved: String,
}

/// The plugin handle. One instance, managed by Tauri.
pub struct Push<R: Runtime> {
    #[cfg(target_os = "android")]
    handle: PluginHandle<R>,
    // see the note on `Secrets`: a `PhantomData<R>` would only be `Send + Sync` when `R` is, and
    // managed state must be both
    #[cfg(not(target_os = "android"))]
    marker: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> Push<R> {
    /// Every app on the phone that can act as a distributor, and the one already chosen.
    ///
    /// An empty list is the ordinary case for someone who has never installed one, and is not an
    /// error: Shiver simply cannot be woken, and should say so rather than pretending it can.
    pub fn distributors(&self) -> Result<(Vec<String>, Option<String>)> {
        #[cfg(target_os = "android")]
        {
            let response = self
                .handle
                .run_mobile_plugin::<DistributorsResponse>("listDistributors", ())
                .map_err(|error| Error::Plugin(error.to_string()))?;

            let saved = (!response.saved.is_empty()).then_some(response.saved);

            return Ok((response.distributors, saved));
        }

        #[cfg(not(target_os = "android"))]
        Ok((Vec::new(), None))
    }

    /// Remembers which distributor to talk to. Registration is a separate step.
    pub fn set_distributor(&self, distributor: &str) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            return self
                .handle
                .run_mobile_plugin::<serde_json::Value>(
                    "setDistributor",
                    serde_json::json!({ "distributor": distributor }),
                )
                .map(|_| ())
                .map_err(|error| Error::Plugin(error.to_string()));
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = distributor;

            Ok(())
        }
    }

    /// Asks for an endpoint for one rail entry.
    ///
    /// The endpoint does not come back from this call — the distributor answers with a broadcast,
    /// which arrives on the event channel. `name` is stored beside the token so the cold-start
    /// receiver can name the server without a network call.
    pub fn register(&self, entry_id: &str, name: &str) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            return self
                .handle
                .run_mobile_plugin::<serde_json::Value>(
                    "register",
                    serde_json::json!({ "token": entry_id, "name": name }),
                )
                .map(|_| ())
                .map_err(|error| Error::Plugin(error.to_string()));
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = (entry_id, name);

            Ok(())
        }
    }

    /// Stops this entry being woken. Safe to call for one that was never registered.
    pub fn unregister(&self, entry_id: &str) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            return self
                .handle
                .run_mobile_plugin::<serde_json::Value>(
                    "unregister",
                    serde_json::json!({ "token": entry_id }),
                )
                .map(|_| ())
                .map_err(|error| Error::Plugin(error.to_string()));
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = entry_id;

            Ok(())
        }
    }

    /// Where distributor events should be delivered.
    ///
    /// Called once at startup. Until it is, the Kotlin side drops what arrives rather than queueing
    /// it — a push Shiver was not ready for is one the next connection will find anyway.
    #[cfg(target_os = "android")]
    pub fn on_event<F>(&self, handler: F) -> Result<()>
    where
        F: Fn(PushEvent) + Send + Sync + 'static,
    {
        // The parameter is what the channel *sends*, which is nothing: traffic here is one-way,
        // from the distributor's broadcast into Rust. It still has to be named.
        let channel: tauri::ipc::Channel<serde_json::Value> =
            tauri::ipc::Channel::new(move |message| {
                // A malformed event is dropped rather than propagated: this arrives from the
                // platform, and the only thing worse than missing a wake-up is a panic here.
                if let Ok(raw) = message.deserialize::<serde_json::Value>() {
                    if let Ok(event) = serde_json::from_value::<PushEvent>(raw) {
                        handler(event);
                    }
                }

                Ok(())
            });

        self.handle
            .run_mobile_plugin::<serde_json::Value>(
                "setEventHandler",
                serde_json::json!({ "handler": channel }),
            )
            .map(|_| ())
            .map_err(|error| Error::Plugin(error.to_string()))
    }

    #[cfg(not(target_os = "android"))]
    pub fn on_event<F>(&self, handler: F) -> Result<()>
    where
        F: Fn(PushEvent) + Send + Sync + 'static,
    {
        let _ = handler;

        Ok(())
    }
}

pub trait PushExt<R: Runtime> {
    fn shiver_push(&self) -> &Push<R>;
}

impl<R: Runtime, T: Manager<R>> PushExt<R> for T {
    fn shiver_push(&self) -> &Push<R> {
        self.state::<Push<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("shiver-push")
        .setup(|app, _api| {
            #[cfg(target_os = "android")]
            {
                let handle = _api.register_android_plugin(PLUGIN_IDENTIFIER, "PushPlugin")?;

                app.manage(Push { handle });
            }

            #[cfg(not(target_os = "android"))]
            {
                app.manage(Push::<R> {
                    marker: std::marker::PhantomData,
                });
            }

            Ok(())
        })
        .build()
}
