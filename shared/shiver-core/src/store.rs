//! The registry file (`servers.json`): the rail, folders and settings. Nothing secret lives here.
//!
//! - An edit runs against a copy, which replaces the registry only once it is on disk. A copy that
//!   serialises to what is already there is not written again (pages report state every second).
//! - Edits are serialised by their own lock; the registry lock is held only to copy and to swap, so
//!   readers never wait on the disk. Writes go to a temp file that is synced and renamed into place.

use std::{
    fs,
    io::Write,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::{de::DeserializeOwned, Serialize};

use crate::error::{Error, Result};

const REGISTRY_FILE: &str = "servers.json";

/// The opt-in message cache of an earlier version, deleted on sight: nothing reads it any more.
const LEGACY_CACHE_FILE: &str = "messages.json";

/// `lock()` that recovers from poisoning, since there is nothing better to recover to.
pub trait LockExt<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Read access to the registry. Changes go through [`Store::edit`], which also persists them.
pub struct ReadGuard<'a, R>(MutexGuard<'a, R>);

impl<R> Deref for ReadGuard<'_, R> {
    type Target = R;

    fn deref(&self) -> &R {
        &self.0
    }
}

pub struct Store<R> {
    path: PathBuf,
    registry: Mutex<R>,
    written: Mutex<String>,
}

fn remove_legacy_cache(dir: &Path) {
    match fs::remove_file(dir.join(LEGACY_CACHE_FILE)) {
        Ok(()) => eprintln!("[shiver] removed the message cache left by an earlier version"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("[shiver] could not remove the old message cache: {error}"),
    }
}

fn to_json(registry: &impl Serialize) -> Result<String> {
    serde_json::to_string_pretty(registry)
        .map_err(|error| Error::Storage(format!("Could not save your servers ({error})")))
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

impl<R> Store<R>
where
    R: Default + Clone + Serialize + DeserializeOwned,
{
    /// Loads `servers.json` from `dir`. A missing file is a first launch; a corrupt one is moved
    /// aside (never overwritten) and the store starts empty; an unreadable one is an error rather
    /// than something to replace with an empty registry.
    pub fn load(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).map_err(|error| {
            Error::Storage(format!(
                "Shiver's settings folder could not be made ({error})"
            ))
        })?;

        remove_legacy_cache(dir);

        let path = dir.join(REGISTRY_FILE);

        let registry = match fs::read_to_string(&path) {
            // editors on Windows add a BOM, which serde rejects
            Ok(contents) => serde_json::from_str(contents.trim_start_matches('\u{feff}'))
                .unwrap_or_else(|error| {
                    let kept = path.with_extension(format!("json.corrupt-{}", unix_seconds()));
                    let _ = fs::rename(&path, &kept);

                    eprintln!(
                        "[shiver] servers.json could not be parsed ({error}), starting empty; the old file is {}",
                        kept.display()
                    );

                    R::default()
                }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => R::default(),
            Err(error) => {
                return Err(Error::Storage(format!(
                    "{REGISTRY_FILE} exists but could not be read ({error})"
                )))
            }
        };

        Ok(Self {
            path,
            written: Mutex::new(to_json(&registry)?),
            registry: Mutex::new(registry),
        })
    }

    pub fn registry(&self) -> ReadGuard<'_, R> {
        ReadGuard(self.registry.locked())
    }

    /// Runs `edit` on a copy; on `Ok` the copy is written to disk and then replaces the registry.
    ///
    /// Generic over the caller's error type so each client keeps its own.
    pub fn edit<T, E>(
        &self,
        edit: impl FnOnce(&mut R) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        let mut written = self.written.locked();
        let mut draft = self.registry.locked().clone();
        let value = edit(&mut draft)?;
        let json = to_json(&draft)?;

        if *written != json {
            self.persist(&json)?;
            *written = json;
        }

        *self.registry.locked() = draft;

        Ok(value)
    }

    fn persist(&self, json: &str) -> Result<()> {
        let path = &self.path;
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));

        let write = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&temp)?;

            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temp, path)?;

            // make the rename itself durable
            #[cfg(unix)]
            if let Some(dir) = path.parent() {
                fs::File::open(dir)?.sync_all()?;
            }

            Ok(())
        })();

        write.map_err(|error| {
            let _ = fs::remove_file(&temp);

            Error::Storage(format!("Could not save your servers ({error})"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde::Deserialize;

    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    struct Registry {
        servers: Vec<String>,
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shiver-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        dir
    }

    #[test]
    fn a_failed_edit_leaves_the_registry_untouched() {
        let dir = temp_dir("failed-edit");
        let store: Store<Registry> = Store::load(&dir).unwrap();

        let failed = store.edit(|registry| {
            registry.servers.push("first".into());

            Err::<(), _>(Error::Storage("no such folder".into()))
        });

        assert!(failed.is_err());
        assert!(store.registry().servers.is_empty());
        assert!(!dir.join(REGISTRY_FILE).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_registry_survives_a_round_trip_with_nothing_left_behind() {
        let dir = temp_dir("round-trip");
        let store: Store<Registry> = Store::load(&dir).unwrap();

        store
            .edit(|registry| {
                registry.servers.push("one".into());

                Ok::<_, Error>(())
            })
            .unwrap();

        let reopened: Store<Registry> = Store::load(&dir).unwrap();

        assert_eq!(*reopened.registry(), *store.registry());
        assert!(fs::read_dir(&dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp")));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_edit_that_changes_nothing_is_not_written() {
        let dir = temp_dir("unchanged");
        let store: Store<Registry> = Store::load(&dir).unwrap();

        store.edit(|_| Ok::<_, Error>(())).unwrap();
        assert!(!dir.join(REGISTRY_FILE).exists());

        store
            .edit(|registry| {
                registry.servers.push("one".into());

                Ok::<_, Error>(())
            })
            .unwrap();
        fs::remove_file(dir.join(REGISTRY_FILE)).unwrap();
        store.edit(|_| Ok::<_, Error>(())).unwrap();
        assert!(!dir.join(REGISTRY_FILE).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_writers_leave_the_newest_registry_on_disk() {
        let dir = temp_dir("race");
        let store = std::sync::Arc::new(Store::<Registry>::load(&dir).unwrap());

        let writers: Vec<_> = (0..8)
            .map(|index| {
                let store = store.clone();

                std::thread::spawn(move || {
                    for round in 0..25 {
                        store
                            .edit(|registry| {
                                registry.servers.push(format!("{index}-{round}"));

                                Ok::<_, Error>(())
                            })
                            .unwrap();
                    }
                })
            })
            .collect();

        for writer in writers {
            writer.join().unwrap();
        }

        let reopened: Store<Registry> = Store::load(&dir).unwrap();

        assert_eq!(reopened.registry().servers.len(), 200);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_that_fails_leaves_the_registry_as_it_was() {
        let dir = temp_dir("failed-write");
        let store: Store<Registry> = Store::load(&dir).unwrap();

        // a directory where the temp file goes makes the write fail
        fs::create_dir_all(dir.join(format!("servers.json.{}.tmp", std::process::id()))).unwrap();

        let failed = store.edit(|registry| {
            registry.servers.push("lost".into());

            Ok::<_, Error>(())
        });

        assert!(failed.is_err());
        assert!(store.registry().servers.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_and_an_unreadable_one_is_an_error() {
        let dir = temp_dir("corrupt");

        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(REGISTRY_FILE), "{ not json").unwrap();

        let store: Store<Registry> = Store::load(&dir).unwrap();

        assert!(store.registry().servers.is_empty());
        assert!(fs::read_dir(&dir).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("corrupt")));

        // a directory where the file should be cannot be read as one
        fs::create_dir_all(dir.join(REGISTRY_FILE)).unwrap();
        assert!(Store::<Registry>::load(&dir).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
