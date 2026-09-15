use std::{
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use tauri::{AppHandle, Manager};

use crate::{
    error::{Error, Result},
    model::Registry,
};

const REGISTRY_FILE: &str = "servers.json";

/// Written by the opt-in message cache, which no longer exists.
const LEGACY_CACHE_FILE: &str = "messages.json";

/// Deletes the message cache left by an earlier version.
///
/// It held the last messages of every channel the user had opened, unencrypted, because that is
/// what they opted into at the time. The feature is gone, so nothing will ever read the file again
/// and leaving it on disk is a copy of their conversations kept for no reason at all.
fn remove_legacy_cache(dir: &std::path::Path) {
    match fs::remove_file(dir.join(LEGACY_CACHE_FILE)) {
        Ok(()) => eprintln!("[shiver] removed the message cache left by an earlier version"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("[shiver] could not remove the old message cache: {error}"),
    }
}

/// The server rail, its folders and the global settings. Everything here is non-secret and lives
/// in a plain json file; credentials go to the OS keychain instead (see `secrets`).
pub struct Store {
    path: PathBuf,
    registry: Mutex<Registry>,
}

impl Store {
    /// An in-memory store, for tests. Never written to disk — the path is one it will not reach.
    #[cfg(test)]
    pub(crate) fn for_tests(registry: Registry) -> Self {
        Self {
            path: PathBuf::from("shiver-tests-never-written.json"),
            registry: Mutex::new(registry),
        }
    }

    pub fn load(app: &AppHandle) -> Result<Self> {
        let dir = app
            .path()
            .app_config_dir()
            .map_err(|error| Error::Storage(error.to_string()))?;

        fs::create_dir_all(&dir).map_err(|error| Error::Storage(error.to_string()))?;

        remove_legacy_cache(&dir);

        let path = dir.join(REGISTRY_FILE);

        // a missing file is a first launch, but an unreadable or corrupt one is not something to
        // silently overwrite: keep the original and start empty so nothing is lost
        let registry = match fs::read_to_string(&path) {
            // windows editors and powershell happily add a utf-8 bom, which serde rejects as
            // trailing garbage before the document even starts
            Ok(contents) => serde_json::from_str(contents.trim_start_matches('\u{feff}')).unwrap_or_else(|error| {
                let _ = fs::rename(&path, path.with_extension("json.corrupt"));

                eprintln!("[shiver] servers.json could not be parsed ({error}), starting empty. The old file was kept as servers.json.corrupt");

                Registry::default()
            }),
            Err(_) => Registry::default(),
        };

        Ok(Self {
            path,
            registry: Mutex::new(registry),
        })
    }

    pub fn registry(&self) -> MutexGuard<'_, Registry> {
        // a panic while holding the lock would leave the registry unreadable for the rest of the
        // session, and there is nothing to recover to, so take the inner value either way
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs `edit` against the registry and persists the result. The lock is released before the
    /// write so a slow disk cannot block unrelated reads.
    pub fn update<T>(&self, edit: impl FnOnce(&mut Registry) -> Result<T>) -> Result<T> {
        let (value, snapshot) = {
            let mut registry = self.registry();
            let value = edit(&mut registry)?;

            (value, registry.clone())
        };

        self.persist(&snapshot)?;

        Ok(value)
    }

    fn persist(&self, registry: &Registry) -> Result<()> {
        let json = serde_json::to_string_pretty(registry)
            .map_err(|error| Error::Storage(error.to_string()))?;

        // write to a sibling first so a crash mid-write cannot truncate the real file
        let temp = self.path.with_extension("json.tmp");

        fs::write(&temp, json).map_err(|error| Error::Storage(error.to_string()))?;
        fs::rename(&temp, &self.path).map_err(|error| Error::Storage(error.to_string()))?;

        Ok(())
    }
}
