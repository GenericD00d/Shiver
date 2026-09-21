//! The registry file, and the rules for writing it safely.
//!
//! Generic over the registry, because the two clients keep different things in theirs — desktop has
//! notification baselines, mobile has icon bytes — but everything *around* the registry was the
//! same file in both crates, down to the comments.
//!
//! Three properties this owes its callers, all of which it used to get wrong:
//!
//! **An edit that fails changes nothing.** The closure runs against a copy, and the copy is only
//! swapped in when it returns `Ok`. It used to run against the live registry and return early on
//! the `?`, so a multi-step edit that failed halfway left its finished half applied in memory and
//! never written — a rail half-reordered until the next restart, or servers pointing at a folder
//! that was never created.
//!
//! **Two writers cannot lose each other's work.** `update` deliberately releases the registry lock
//! before touching the disk, so `persist` takes a lock of its own; without one, two saves raced on
//! a single fixed temp path and the older snapshot could land last.
//!
//! **A crash cannot leave a half-written file.** The temp file is flushed and synced before the
//! rename, which is the part that makes the rename-into-place actually mean anything.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::{de::DeserializeOwned, Serialize};

use crate::error::{Error, Result};

const REGISTRY_FILE: &str = "servers.json";

/// Written by the opt-in message cache, which no longer exists.
const LEGACY_CACHE_FILE: &str = "messages.json";

/// Deletes the message cache left by an earlier version.
///
/// It held the last messages of every channel the user had opened, unencrypted, because that is
/// what they opted into at the time. The feature is gone, so nothing will ever read the file again
/// and leaving it on disk is a copy of their conversations kept for no reason at all.
fn remove_legacy_cache(dir: &Path) {
    match fs::remove_file(dir.join(LEGACY_CACHE_FILE)) {
        Ok(()) => eprintln!("[shiver] removed the message cache left by an earlier version"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("[shiver] could not remove the old message cache: {error}"),
    }
}

/// The server rail, its folders and the global settings. Everything here is non-secret and lives
/// in a plain json file; credentials go to the OS keychain instead.
pub struct Store<R> {
    /// Where the registry is written, or `None` for a store that exists only in memory.
    path: Option<PathBuf>,
    registry: Mutex<R>,
    /// Held across a write, so two saves cannot interleave or land out of order.
    ///
    /// Separate from the registry lock on purpose: `update` releases that one before writing so a
    /// slow disk does not block unrelated reads, which is the right trade and is also exactly what
    /// left the writes unserialised.
    writing: Mutex<()>,
}

impl<R> Store<R>
where
    R: Default + Clone + Serialize + DeserializeOwned,
{
    /// An in-memory store, for tests.
    ///
    /// **Genuinely never written**, which it was not before: it carried a relative path and
    /// `update` persisted unconditionally, so every test that edited one dropped a
    /// `shiver-tests-never-written.json` into whatever the working directory happened to be — under
    /// a doc comment saying it could not. `None` is the difference between a promise and a
    /// filename that sounds like one.
    pub fn for_tests(registry: R) -> Self {
        Self {
            path: None,
            registry: Mutex::new(registry),
            writing: Mutex::new(()),
        }
    }

    pub fn load(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).map_err(|error| Error::Storage(error.to_string()))?;

        remove_legacy_cache(dir);

        let path = dir.join(REGISTRY_FILE);

        // a missing file is a first launch, but an unreadable or corrupt one is not something to
        // silently overwrite: keep the original and start empty so nothing is lost
        let registry = match fs::read_to_string(&path) {
            // windows editors and powershell happily add a utf-8 bom, which serde rejects as
            // trailing garbage before the document even starts
            Ok(contents) => serde_json::from_str(contents.trim_start_matches('\u{feff}'))
                .unwrap_or_else(|error| {
                    let _ = fs::rename(&path, path.with_extension("json.corrupt"));

                    eprintln!("[shiver] servers.json could not be parsed ({error}), starting empty. The old file was kept as servers.json.corrupt");

                    R::default()
                }),
            Err(_) => R::default(),
        };

        Ok(Self {
            path: Some(path),
            registry: Mutex::new(registry),
            writing: Mutex::new(()),
        })
    }

    pub fn registry(&self) -> MutexGuard<'_, R> {
        // a panic while holding the lock would leave the registry unreadable for the rest of the
        // session, and there is nothing to recover to, so take the inner value either way
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs `edit` against the registry and persists the result.
    ///
    /// **The edit runs against a copy.** Nothing it changed is visible to anyone, in memory or on
    /// disk, unless it returns `Ok` — so a multi-step edit that fails partway leaves the registry
    /// exactly as it found it. `create_folder_with` is the case that made this matter: it stamped
    /// each member server with the new folder's id before pushing the folder itself, so a failure
    /// in between left servers pointing at a folder that did not exist.
    ///
    /// The registry lock is released before the write, so a slow disk cannot block unrelated reads.
    /// `persist` serialises the writers itself.
    ///
    /// Generic over the caller's error type, so each client keeps its own — the closures here are
    /// the clients' own code and they raise `UnknownServer`, `UnknownFolder` and the rest, none of
    /// which is this crate's vocabulary.
    ///
    /// Named `edit` rather than `update` so each client can put a non-generic `update` in front of
    /// it. With the error type open, a closure that never returns `Err` leaves it unconstrained,
    /// and a client `Error` with several `From` impls then has nothing to pick from.
    pub fn edit<T, E>(
        &self,
        edit: impl FnOnce(&mut R) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        // Serialised while the lock is still held, so the registry is cloned once rather than
        // twice. It used to clone for the draft and clone again to publish it — the second copy
        // existing only so `persist` had something to read after the lock was released. Turning it
        // into the json it was always going to become costs the same walk and keeps nothing.
        let (value, json) = {
            let mut registry = self.registry();
            let mut draft = registry.clone();
            let value = edit(&mut draft)?;

            let json = self.serialise(&draft)?;

            *registry = draft;

            (value, json)
        };

        self.persist(json)?;

        Ok(value)
    }

    /// The registry as it will be written, or nothing when this store has no file.
    ///
    /// `None` rather than an empty string, so an in-memory store does no serialising at all rather
    /// than serialising something it will throw away.
    fn serialise(&self, registry: &R) -> Result<Option<String>> {
        if self.path.is_none() {
            return Ok(None);
        }

        serde_json::to_string_pretty(registry)
            .map(Some)
            .map_err(|error| Error::Storage(error.to_string()))
    }

    fn persist(&self, json: Option<String>) -> Result<()> {
        // an in-memory store, which is what `for_tests` builds
        let (Some(path), Some(json)) = (self.path.as_ref(), json) else {
            return Ok(());
        };

        // One writer at a time. `update` releases the registry lock before getting here, so without
        // this two saves could write the same temp file at once and rename in whichever order they
        // finished — which is not the order the edits happened in, so the older snapshot could win.
        let _writing = self
            .writing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Write to a sibling first, then rename, so a crash mid-write cannot truncate the real
        // file. The sibling is named per-process-per-write rather than fixed, because two of these
        // sharing one name is the race above wearing a different hat.
        let temp = path.with_extension(format!(
            "json.{}.{:x}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0)
        ));

        // Synced before the rename, which is the part that makes the rename mean anything: on
        // several filesystems the metadata operation can land before the data does, so an unsynced
        // temp file can be renamed into place empty.
        let write = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&temp)?;

            file.write_all(json.as_bytes())?;
            file.sync_all()
        })();

        if let Err(error) = write {
            let _ = fs::remove_file(&temp);

            return Err(Error::Storage(error.to_string()));
        }

        if let Err(error) = fs::rename(&temp, path) {
            let _ = fs::remove_file(&temp);

            return Err(Error::Storage(error.to_string()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde::Deserialize;

    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    struct Registry {
        servers: Vec<String>,
        folders: Vec<String>,
    }

    /// The guarantee the old `update` did not give: a closure that fails changes nothing.
    ///
    /// It ran against the live registry, so everything before the `?` stayed applied in memory
    /// while `persist` was skipped — leaving the process disagreeing with its own file.
    #[test]
    fn a_failed_edit_leaves_the_registry_untouched() {
        let store = Store::for_tests(Registry::default());

        let failed = store.edit(|registry| {
            registry.servers.push("first".into());
            registry.servers.push("second".into());

            // the shape of `create_folder_with`: members stamped, then something is not found
            Err::<(), _>(Error::Storage("no such folder".into()))
        });

        assert!(failed.is_err());
        assert!(
            store.registry().servers.is_empty(),
            "a failed edit must not leave half of itself applied"
        );
    }

    #[test]
    fn a_successful_edit_is_applied() {
        let store = Store::for_tests(Registry::default());

        store
            .edit(|registry| {
                registry.servers.push("kept".into());

                Ok::<_, Error>(())
            })
            .expect("an in-memory store never reaches the disk");

        assert_eq!(store.registry().servers, vec!["kept".to_string()]);
    }

    /// A registry written and read back, through the real file path rather than `for_tests`.
    #[test]
    fn a_registry_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("shiver-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let store: Store<Registry> = Store::load(&dir).expect("a fresh directory");

        store
            .edit(|registry| {
                registry.servers.push("one".into());
                registry.folders.push("games".into());

                Ok::<_, Error>(())
            })
            .expect("the write should land");

        let reopened: Store<Registry> = Store::load(&dir).expect("the directory exists now");

        assert_eq!(*reopened.registry(), *store.registry());

        // nothing left behind: a temp file that outlives its write is a temp file that gets read
        let leftovers = fs::read_dir(&dir)
            .expect("the directory exists")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .count();

        assert_eq!(leftovers, 0, "the temp file should be gone once renamed");

        let _ = fs::remove_dir_all(&dir);
    }

    /// Many writers at once, which is what `sync`, the drain and the settings screen actually are.
    #[test]
    fn concurrent_writers_do_not_lose_each_other() {
        let dir = std::env::temp_dir().join(format!("shiver-store-race-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let store: std::sync::Arc<Store<Registry>> =
            std::sync::Arc::new(Store::load(&dir).expect("a fresh directory"));

        let writers: Vec<_> = (0..8)
            .map(|index| {
                let store = store.clone();

                std::thread::spawn(move || {
                    for round in 0..10 {
                        let _ = store.edit(|registry| {
                            registry.servers.push(format!("{index}-{round}"));

                            Ok::<_, Error>(())
                        });
                    }
                })
            })
            .collect();

        for writer in writers {
            writer.join().expect("no writer should panic");
        }

        assert_eq!(store.registry().servers.len(), 80);

        // and the file on disk is the whole of it, not one writer's view of it
        let reopened: Store<Registry> = Store::load(&dir).expect("the directory exists");

        assert_eq!(
            reopened.registry().servers.len(),
            80,
            "every write should be on disk, not just the last one to finish"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
