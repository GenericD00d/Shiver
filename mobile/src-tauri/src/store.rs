//! This client's registry, kept by the shared store.
//!
//! The store itself lives in `shared/shiver-core`: everything around the registry — the corrupt-file
//! handling, the legacy cache cleanup, the atomic write — was the same file in both clients, so it
//! is one file now. What stays here is the only part that differs: where this platform puts it.

use tauri::{AppHandle, Manager};

use crate::{error::Error, model::Registry};

pub type Store = shiver_core::Store<Registry>;

/// This client's `update`, with the error type pinned to this client's own.
///
/// The shared `edit` is generic over the error so each client keeps its own vocabulary. That is the
/// right shape there and the wrong one at the call sites: a closure that only ever succeeds leaves
/// the error type unconstrained, and `Error` has several `From` impls for inference to choose
/// between. Fixing it here means every caller reads exactly as it did before.
pub trait RegistryStore {
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
        .map_err(|error| Error::Storage(error.to_string()))?;

    Ok(Store::load(&dir)?)
}
