//! UnifiedPush wake-ups for Android (a no-op elsewhere).
//!
//! The core registers one UnifiedPush instance per rail entry under that entry's push token; the
//! distributor answers with broadcasts that arrive as [`PushEvent`]s. The companion plugin posts an
//! empty body to an endpoint when a message arrives for an away user, and Shiver then fetches what
//! happened over its own connection. No commands are exposed to any webview: an endpoint is a
//! capability to wake the phone.

use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Plugin(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A distributor broadcast, as the Kotlin side reports it.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PushEvent {
    Endpoint {
        token: String,
        endpoint: String,
    },
    /// something arrived for this token; carries nothing else by design
    Message {
        token: String,
    },
    /// usually no distributor has been set up
    Failed {
        token: String,
    },
    Unregistered {
        token: String,
    },
}

#[derive(Default, Deserialize)]
struct DistributorsResponse {
    distributors: Vec<String>,
    /// the chosen one, or empty
    saved: String,
}

pub struct Push<R: Runtime> {
    #[cfg(target_os = "android")]
    handle: tauri::plugin::PluginHandle<R>,
    /// `fn() -> R` keeps the state `Send + Sync` whatever `R` is
    #[cfg(not(target_os = "android"))]
    marker: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> Push<R> {
    #[cfg(target_os = "android")]
    fn call<T: DeserializeOwned + Default>(&self, command: &str, payload: Value) -> Result<T> {
        self.handle
            .run_mobile_plugin(command, payload)
            .map_err(|error| Error::Plugin(error.to_string()))
    }

    #[cfg(not(target_os = "android"))]
    fn call<T: DeserializeOwned + Default>(&self, _command: &str, _payload: Value) -> Result<T> {
        Ok(T::default())
    }

    /// Installed distributors, and the chosen one. An empty list (none installed) is not an error.
    pub fn distributors(&self) -> Result<(Vec<String>, Option<String>)> {
        let response: DistributorsResponse = self.call("listDistributors", Value::Null)?;

        Ok((
            response.distributors,
            Some(response.saved).filter(|saved| !saved.is_empty()),
        ))
    }

    pub fn set_distributor(&self, distributor: &str) -> Result<()> {
        self.call::<Value>("setDistributor", json!({ "distributor": distributor }))
            .map(drop)
    }

    /// Asks for an endpoint for `token`; it arrives later as a [`PushEvent::Endpoint`]. `name` is
    /// kept for the cold-start notification.
    pub fn register(&self, token: &str, name: &str) -> Result<()> {
        self.call::<Value>("register", json!({ "token": token, "name": name }))
            .map(drop)
    }

    /// Safe for a token that was never registered.
    pub fn unregister(&self, token: &str) -> Result<()> {
        self.call::<Value>("unregister", json!({ "token": token }))
            .map(drop)
    }

    /// Where distributor events go. Until this is called they are dropped (the next connection
    /// finds whatever they were about).
    pub fn on_event<F>(&self, handler: F) -> Result<()>
    where
        F: Fn(PushEvent) + Send + Sync + 'static,
    {
        #[cfg(target_os = "android")]
        {
            let channel = tauri::ipc::Channel::<Value>::new(move |message| {
                // malformed platform events are dropped, never propagated
                if let Ok(event) = message.deserialize::<PushEvent>() {
                    handler(event);
                }

                Ok(())
            });

            self.call::<Value>("setEventHandler", json!({ "handler": channel }))
                .map(drop)
        }

        #[cfg(not(target_os = "android"))]
        {
            drop(handler);

            Ok(())
        }
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
            app.manage(Push {
                handle: _api.register_android_plugin("com.shiver.push", "PushPlugin")?,
            });

            #[cfg(not(target_os = "android"))]
            app.manage(Push::<R> {
                marker: std::marker::PhantomData,
            });

            Ok(())
        })
        .build()
}
