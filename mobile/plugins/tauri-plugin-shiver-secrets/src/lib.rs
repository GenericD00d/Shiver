//! Encrypted at-rest storage on Android (`EncryptedSharedPreferences` under a Keystore master key)
//! for sessions, remembered passwords and carried page state, plus wiping an origin's web storage.
//! `keyring` has no Android backend, hence this plugin.
//!
//! No commands are exposed to any webview; the core is the only caller. On other targets (where
//! the mobile crate's tests run) nothing is kept, rather than keeping a token unprotected.

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

/// Empty rather than absent for a missing key.
#[derive(Default, Deserialize)]
struct ValueResponse {
    value: String,
}

#[derive(Default, Deserialize)]
struct KeysResponse {
    keys: Vec<String>,
}

pub struct Secrets<R: Runtime> {
    #[cfg(target_os = "android")]
    handle: tauri::plugin::PluginHandle<R>,
    /// `fn() -> R` keeps the state `Send + Sync` whatever `R` is
    #[cfg(not(target_os = "android"))]
    marker: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> Secrets<R> {
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

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.call::<Value>("setSecret", json!({ "key": key, "value": value }))
            .map(drop)
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let response: ValueResponse = self.call("getSecret", json!({ "key": key }))?;

        Ok(Some(response.value).filter(|value| !value.is_empty()))
    }

    pub fn remove(&self, key: &str) -> Result<()> {
        self.call::<Value>("removeSecret", json!({ "key": key }))
            .map(drop)
    }

    /// Every key held.
    pub fn keys(&self) -> Result<Vec<String>> {
        Ok(self.call::<KeysResponse>("secretKeys", json!({}))?.keys)
    }

    /// Deletes an origin's web storage and cookies from the platform side (removing keys from a
    /// page leaves the old bytes in the webview's LevelDB log).
    pub fn wipe_origin(&self, origin: &str) -> Result<()> {
        self.call::<Value>("wipeOrigin", json!({ "origin": origin }))
            .map(drop)
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
        .setup(|app, _api| {
            #[cfg(target_os = "android")]
            app.manage(Secrets {
                handle: _api.register_android_plugin("com.shiver.secrets", "SecretsPlugin")?,
            });

            #[cfg(not(target_os = "android"))]
            app.manage(Secrets::<R> {
                marker: std::marker::PhantomData,
            });

            Ok(())
        })
        .build()
}
