//! This client's registry store: the shared store with the error type pinned to this crate's.

use tauri::{AppHandle, Manager};

use crate::{
    error::{Core, Error},
    model::Registry,
};

pub type Store = shiver_core::Store<Registry>;

pub trait RegistryStore {
    /// Edits a copy of the registry and persists it if the closure returns `Ok`.
    fn update<T>(&self, edit: impl FnOnce(&mut Registry) -> Result<T, Error>) -> Result<T, Error>;
}

impl RegistryStore for Store {
    fn update<T>(&self, edit: impl FnOnce(&mut Registry) -> Result<T, Error>) -> Result<T, Error> {
        self.edit(edit)
    }
}

/// Opens the registry in this platform's config directory.
pub fn load(app: &AppHandle) -> Result<Store, Error> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| Core::Storage(format!("Shiver's settings folder is unknown ({error})")))?;

    Ok(Store::load(&dir)?)
}
