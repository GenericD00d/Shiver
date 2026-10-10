//! Each server's browser profile (its cookies and storage), and clearing out the ones no longer in
//! use: WebView2 profile folders on Windows, WebKitGTK data directories elsewhere.

use std::{path::PathBuf, time::Duration};

use tauri::{AppHandle, Manager};

use crate::{model::ServerEntry, store::Store};

pub(super) fn profiles_root(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_local_data_dir()
        .ok()
        .map(|dir| dir.join("webviews"))
}

/// The directory name of an entry's current browser profile. Each entry has its own profile (two
/// accounts on one server must not share `localStorage`), and logging out moves to a new one.
pub(super) fn profile_name(entry: &ServerEntry) -> String {
    match &entry.profile {
        Some(generation) => format!("{}-{generation}", entry.id),
        None => entry.id.clone(),
    }
}

/// The same profile on macOS (14+), which keeps page storage in a data store named by a uuid
/// rather than a directory: the generation's, else the entry's.
pub(super) fn data_store(entry: &ServerEntry) -> [u8; 16] {
    uuid::Uuid::parse_str(entry.profile.as_deref().unwrap_or(&entry.id))
        .map_or([0; 16], uuid::Uuid::into_bytes)
}

/// Drops every data store no current entry's profile uses. Their pages must be closed first.
#[cfg(target_vendor = "apple")]
fn prune_data_stores(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        let current: std::collections::HashSet<[u8; 16]> = app
            .state::<Store>()
            .registry()
            .servers
            .iter()
            .map(data_store)
            .collect();
        let stores = app.fetch_data_store_identifiers().await.unwrap_or_default();

        for store in stores.into_iter().filter(|store| !current.contains(store)) {
            if let Err(error) = app.remove_data_store(store).await {
                eprintln!("[shiver] could not remove a page's data store: {error}");
            }
        }
    });
}

/// Deletes profile directories in the background, retrying while WebView2 releases its files.
/// Anything still left is removed by `prune_profiles` on the next launch.
fn delete_dirs_later(dirs: Vec<PathBuf>) {
    if dirs.is_empty() {
        return;
    }

    std::thread::spawn(move || {
        for _ in 0..20 {
            let remaining: Vec<&PathBuf> = dirs
                .iter()
                .filter(|dir| match std::fs::remove_dir_all(dir) {
                    Ok(()) => false,
                    Err(error) => error.kind() != std::io::ErrorKind::NotFound,
                })
                .collect();

            if remaining.is_empty() {
                return;
            }

            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Every profile directory belonging to an entry (current and older generations).
fn profile_dirs_of(app: &AppHandle, entry_id: &str, keep: Option<&str>) -> Vec<PathBuf> {
    let Some(root) = profiles_root(app) else {
        return Vec::new();
    };

    std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(|dir| dir.ok())
        .map(|dir| dir.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            (name == entry_id || name.starts_with(&format!("{entry_id}-")))
                && Some(name.as_str()) != keep
        })
        .map(|name| root.join(name))
        .collect()
}

/// Discards all of an entry's browser storage (cookies, localStorage, cache). Its pages must be
/// closed first, and on macOS the registry must already have dropped or replaced its profile.
pub fn discard_profiles(app: &AppHandle, entry_id: &str, keep: Option<&ServerEntry>) {
    let keep = keep.map(profile_name);

    delete_dirs_later(profile_dirs_of(app, entry_id, keep.as_deref()));

    #[cfg(target_vendor = "apple")]
    prune_data_stores(app);
}

/// At launch, deletes profiles that belong to no current entry (removed servers, old generations,
/// deletions that did not finish last time).
pub fn prune_profiles(app: &AppHandle) {
    #[cfg(target_vendor = "apple")]
    prune_data_stores(app);

    let Some(root) = profiles_root(app) else {
        return;
    };

    let current: std::collections::HashSet<String> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .map(profile_name)
        .collect();

    let stale = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(|dir| dir.ok())
        .filter(|dir| !current.contains(dir.file_name().to_string_lossy().as_ref()))
        .map(|dir| dir.path())
        .collect();

    delete_dirs_later(stale);
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiver_core::model::NotifyLevel;

    fn entry(profile: Option<&str>) -> ServerEntry {
        ServerEntry {
            id: "0b6d7e4e-1a4f-4bb0-9d3c-2f3b1a9e8c11".into(),
            origin: "https://chat.example.com".into(),
            name: "Chat".into(),
            icon_url: None,
            identity: None,
            account_label: None,
            folder_id: None,
            position: 0,
            accept_any_size: false,
            profile: profile.map(str::to_string),
            media_allowed: false,
            notify: NotifyLevel::default(),
        }
    }

    #[test]
    fn each_profile_generation_has_its_own_data_store() {
        let first = data_store(&entry(None));
        let later = data_store(&entry(Some("9f1c0d2e6b7a4c3d8e9f0a1b2c3d4e5f")));

        assert_ne!(first, [0; 16]);
        assert_ne!(first, later);
        assert_eq!(
            later,
            data_store(&entry(Some("9f1c0d2e6b7a4c3d8e9f0a1b2c3d4e5f")))
        );
    }
}
