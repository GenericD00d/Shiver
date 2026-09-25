//! Server logos, kept as one `data:` uri per entry in `icons/<entry id>` under the app's data
//! folder rather than in `servers.json`, fetched once and deleted with the entry.

use std::{collections::HashMap, fs, io, path::PathBuf};

use tauri::{AppHandle, Manager};

use crate::store::Store;

fn dir(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_data_dir().ok()?.join("icons"))
}

/// Entry ids are uuids; nothing else names a file.
fn is_entry_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn path(app: &AppHandle, id: &str) -> Option<PathBuf> {
    Some(dir(app).filter(|_| is_entry_id(id))?.join(id))
}

/// Stores an entry's logo, or deletes it when there is none.
pub fn save(app: &AppHandle, id: &str, data: Option<&str>) {
    let Some(path) = path(app, id) else {
        return;
    };

    let result = match data {
        Some(data) => path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&path, data)),
        None => fs::remove_file(&path).or_else(|error| match error.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(error),
        }),
    };

    if let Err(error) = result {
        eprintln!("[shiver] could not store a server's logo: {error}");
    }
}

/// Every stored logo, by entry id.
pub fn load(app: &AppHandle, ids: &[String]) -> HashMap<String, String> {
    ids.iter()
        .filter_map(|id| Some((id.clone(), fs::read_to_string(path(app, id)?).ok()?)))
        .collect()
}

/// At startup: deletes the logos of entries that are gone and fetches those that are missing.
pub fn restore(app: &AppHandle) {
    let servers: Vec<(String, Option<String>)> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .map(|server| (server.id.clone(), server.icon_url.clone()))
        .collect();

    if let Some(Ok(entries)) = dir(app).map(fs::read_dir) {
        for entry in entries.flatten() {
            if !servers
                .iter()
                .any(|(id, _)| entry.file_name() == id.as_str())
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        for (id, url) in servers {
            let missing = path(&app, &id).is_some_and(|path| !path.exists());

            if let (true, Some(url)) = (missing, url) {
                if let Some(data) = shiver_core::probe::fetch_icon(&url).await {
                    save(&app, &id, Some(&data));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::is_entry_id;

    #[test]
    fn only_entry_ids_name_files() {
        assert!(is_entry_id("0f8c2b4e-5d6a-4f3b-9c1d-2e7a8b9c0d1e"));

        for bad in ["", "..", "a/b", "a\\b", "servers.json"] {
            assert!(!is_entry_id(bad), "{bad} must not name a file");
        }
    }
}
