//! Encrypted at-rest storage for the one secret Shiver holds on Android: a server session token.
//!
//! The desktop client keeps sessions in the OS keychain through `keyring`, which has no Android
//! backend. So the mobile client held nothing, and its core connection had to scrape a token out of
//! whichever server page happened to be on screen — which made the unified inbox depend on the user
//! having visited that server since the last cold start, and made it fragile besides.
//!
//! This is the Android answer: `EncryptedSharedPreferences`, whose master key lives in the Android
//! Keystore and is hardware-backed where the device has a secure element. Values are encrypted with
//! AES-256-GCM and keys with AES-256-SIV, so the file reveals neither a token nor which server it
//! belongs to.
//!
//! Nothing here is exposed to a webview. The plugin declares no commands, so Shiver own pages cannot
//! read a token and a server page certainly cannot. The only caller is the core.

use serde::Deserialize;

#[cfg(target_os = "android")]
use tauri::plugin::PluginHandle;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.shiver.secrets";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Plugin(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Deserialize)]
#[allow(dead_code)]
struct ValueResponse {
    /// empty rather than absent: a json null across the bridge would mean the same and travel less
    /// predictably
    value: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct KeysResponse {
    keys: Vec<String>,
}

/// The store itself. One instance, managed by the plugin.
pub struct Secrets<R: Runtime> {
    #[cfg(target_os = "android")]
    handle: PluginHandle<R>,
    // `PhantomData<R>` would only be `Send + Sync` when `R` is, and managed state must be both.
    // A function-pointer marker carries the parameter without borrowing its auto traits.
    #[cfg(not(target_os = "android"))]
    marker: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> Secrets<R> {
    /// Stores a value, replacing any previous one under the same key.
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            return self
                .handle
                .run_mobile_plugin::<serde_json::Value>(
                    "setSecret",
                    serde_json::json!({ "key": key, "value": value }),
                )
                .map(|_| ())
                .map_err(|error| Error::Plugin(error.to_string()));
        }

        // The mobile crate also builds for a desktop host, which is how its tests run. There is no
        // encrypted store there, so this keeps nothing rather than keeping a token unprotected.
        #[cfg(not(target_os = "android"))]
        {
            let _ = (key, value);

            Ok(())
        }
    }

    /// Reads a value back, or `None` when there is none under that key.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        #[cfg(target_os = "android")]
        {
            let response = self
                .handle
                .run_mobile_plugin::<ValueResponse>("getSecret", serde_json::json!({ "key": key }))
                .map_err(|error| Error::Plugin(error.to_string()))?;

            return Ok(Some(response.value).filter(|value| !value.is_empty()));
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = key;

            Ok(None)
        }
    }

    pub fn remove(&self, key: &str) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            return self
                .handle
                .run_mobile_plugin::<serde_json::Value>(
                    "removeSecret",
                    serde_json::json!({ "key": key }),
                )
                .map(|_| ())
                .map_err(|error| Error::Plugin(error.to_string()));
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = key;

            Ok(())
        }
    }

    /// Deletes everything a server's page kept in web storage.
    ///
    /// Separate from the session store above, and the reason this plugin is not only a key-value
    /// box: clearing a key from a page leaves the old bytes in the webview's LevelDB log until it
    /// is compacted, so the only way to be sure a session is off the disk is to drop the origin's
    /// storage from the platform side.
    pub fn wipe_origin(&self, origin: &str) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            return self
                .handle
                .run_mobile_plugin::<serde_json::Value>(
                    "wipeOrigin",
                    serde_json::json!({ "origin": origin }),
                )
                .map(|_| ())
                .map_err(|error| Error::Plugin(error.to_string()));
        }

        #[cfg(not(target_os = "android"))]
        {
            let _ = origin;

            Ok(())
        }
    }

    /// Every key held, so the core can reconnect to each server it has a session for without being
    /// told which those are.
    pub fn keys(&self) -> Result<Vec<String>> {
        #[cfg(target_os = "android")]
        {
            let response = self
                .handle
                .run_mobile_plugin::<KeysResponse>("secretKeys", serde_json::json!({}))
                .map_err(|error| Error::Plugin(error.to_string()))?;

            return Ok(response.keys);
        }

        #[cfg(not(target_os = "android"))]
        Ok(Vec::new())
    }
}

pub trait SecretsExt<R: Runtime> {
    fn shiver_secrets(&self) -> &Secrets<R>;
}

impl<R: Runtime, T: Manager<R>> SecretsExt<R> for T {
    fn shiver_secrets(&self) -> &Secrets<R> {
        self.state::<Secrets<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::<R, ()>::new("shiver-secrets")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            let secrets = Secrets {
                handle: api.register_android_plugin(PLUGIN_IDENTIFIER, "SecretsPlugin")?,
            };

            #[cfg(not(target_os = "android"))]
            let secrets = {
                let _ = api;

                // named, because nothing else in this branch ties the marker to the runtime
                Secrets::<R> {
                    marker: std::marker::PhantomData,
                }
            };

            app.manage(secrets);

            Ok(())
        })
        .build()
}
