//! Noticing a newer release. Android cannot self-update without `REQUEST_INSTALL_PACKAGES`, so this
//! only tells the user (a notification, and the settings screen) and opens the releases page.

use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use shiver_core::LockExt;
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    error::{Core, Error, Result},
    store::{RegistryStore, Store},
};

const MANIFEST: &str = "https://github.com/GenericD00d/Shiver/releases/latest/download/latest.json";
const RELEASES: &str = "https://github.com/GenericD00d/Shiver/releases/latest";
const REPOSITORY: &str = "https://github.com/GenericD00d/Shiver";

/// Tells Shiver's page a newer version was found, so its notice appears without asking on a timer.
const UPDATE_EVENT: &str = "shiver://update";
const FIRST_CHECK: Duration = Duration::from_secs(45);
const NOTIFICATION_ID: i32 = 1;
/// `latest.json` is a few hundred bytes.
const MAX_MANIFEST: usize = 64 * 1024;

#[derive(Deserialize)]
struct Manifest {
    version: String,
}

/// The newer version, once known.
#[derive(Default)]
pub struct Available(Mutex<Option<String>>);

impl Available {
    pub fn get(&self) -> Option<String> {
        self.0.locked().clone()
    }

    fn set(&self, version: Option<String>) {
        *self.0.locked() = version;
    }
}

#[tauri::command]
pub fn update_available(app: AppHandle) -> Option<String> {
    app.state::<Available>().get()
}

#[tauri::command]
pub fn open_releases(app: AppHandle) -> Result<()> {
    open(&app, RELEASES)
}

#[tauri::command]
pub fn open_repository(app: AppHandle) -> Result<()> {
    open(&app, REPOSITORY)
}

fn open(app: &AppHandle, url: &str) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| Error::Webview(error.to_string()))
}

/// Checks now, on request (a skipped version is still reported).
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<String>> {
    let newest = fetch()
        .await
        .ok_or_else(|| Core::Unreachable("GitHub, to check for updates".into()))?;

    if !is_newer(&newest, &app.package_info().version.to_string()) {
        return Ok(None);
    }

    app.state::<Available>().set(Some(newest.clone()));

    Ok(Some(newest))
}

#[tauri::command]
pub async fn skip_update(app: AppHandle, version: String) -> Result<()> {
    app.state::<Store>().update(|registry| {
        registry.settings.skipped_update = Some(version);

        Ok(())
    })?;

    app.state::<Available>().set(None);

    Ok(())
}

/// One check shortly after launch; announces a newer, not-skipped version.
pub fn start(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;

        let Some(newest) = fetch().await else {
            return;
        };

        let skipped = app
            .state::<Store>()
            .registry()
            .settings
            .skipped_update
            .as_deref()
            == Some(newest.as_str());

        if skipped || !is_newer(&newest, &app.package_info().version.to_string()) {
            return;
        }

        app.state::<Available>().set(Some(newest.clone()));
        let _ = app.emit(UPDATE_EVENT, &newest);

        use tauri_plugin_notification::NotificationExt;

        let _ = app
            .notification()
            .builder()
            .id(NOTIFICATION_ID)
            .title(format!("Shiver {newest} is available"))
            .body("Tap to open the releases page.")
            .show();
    });
}

/// The latest release's version. GitHub answers the manifest url with a redirect to its download
/// host, so redirects are followed — but only to GitHub's own hosts — and the body is bounded.
async fn fetch() -> Option<String> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let github = attempt.url().scheme() == "https"
            && attempt.url().host_str().is_some_and(|host| {
                host == "github.com" || host.ends_with(".githubusercontent.com")
            });

        if github && attempt.previous().len() < 5 {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });

    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(policy)
        .build()
        .ok()?
        .get(MANIFEST)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        eprintln!("[shiver] no update manifest: {}", response.status());

        return None;
    }

    let body = shiver_core::http::bytes_within_limit(response, MAX_MANIFEST).await?;

    serde_json::from_slice::<Manifest>(&body)
        .ok()
        .map(|manifest| manifest.version)
}

/// Semantic-version comparison ("0.1.10" > "0.1.9"); unparseable is never newer.
fn is_newer(newest: &str, running: &str) -> bool {
    matches!(
        (semver::Version::parse(newest), semver::Version::parse(running)),
        (Ok(newest), Ok(running)) if newest > running
    )
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn versions_compare_as_versions() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.9", "0.1.10"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(is_newer("0.1.0", "0.1.0-b"));
        assert!(!is_newer("0.1.0-b", "0.1.0"));
        assert!(!is_newer("not a version", "0.1.0"));
    }
}
