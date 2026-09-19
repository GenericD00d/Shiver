//! Telling the phone that a newer Shiver exists.
//!
//! **This notifies; it does not update.** Desktop verifies a minisign signature and installs the
//! thing itself, and none of that is available here: Tauri's updater has no Android implementation,
//! and an app that installs packages needs `REQUEST_INSTALL_PACKAGES` — a permission that would let
//! Shiver install anything at all, which is a poor trade for saving one tap. So the phone is told,
//! and the tap goes to the release page, where Android's own installer does the work and asks the
//! browser for permission rather than Shiver.
//!
//! What protects that download is the APK's own signature: Android refuses an update signed by a
//! different key than the installed copy, so a replacement APK from anywhere else simply will not
//! install over this one.
//!
//! The manifest read here is the same `latest.json` the desktop updater reads. One file describing
//! what the newest version is, rather than two that can disagree.

use crate::store::RegistryStore;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use tauri::{AppHandle, Manager};

use crate::error::Result;

/// The desktop updater's manifest, read for its version alone. The platform entries below it
/// describe Windows installers and are no use here.
const MANIFEST: &str = "https://github.com/GenericD00d/Shiver/releases/latest/download/latest.json";

/// Where the tap goes. `latest` rather than a pinned tag, so this keeps working for whatever is
/// newest without Shiver having to construct a url from a version it just learned.
const RELEASES: &str = "https://github.com/GenericD00d/Shiver/releases/latest";

/// The project itself, for the link in About.
const REPOSITORY: &str = "https://github.com/GenericD00d/Shiver";

/// Long enough after launch to be out of the way of connecting to every server.
const FIRST_CHECK: Duration = Duration::from_secs(45);

/// Android's notification id for this. Fixed, so a second announcement replaces the first rather
/// than stacking — the id space is shared with the per-server ones, which are hashes of entry ids.
const NOTIFICATION_ID: i32 = 1;

#[derive(Deserialize)]
struct Manifest {
    version: String,
}

/// The newer version, once one is known. Read by the settings screen.
///
/// Kept rather than re-fetched: the screen is opened far more often than a release happens, and a
/// settings page that waits on the network to draw a row is a settings page that feels broken.
#[derive(Default)]
pub struct Available(Mutex<Option<String>>);

impl Available {
    pub fn get(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set(&self, version: Option<String>) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = version;
    }
}

/// What the settings screen asks on the way in.
#[tauri::command]
pub fn update_available(app: AppHandle) -> Result<Option<String>> {
    Ok(app.state::<Available>().get())
}

/// Opens the release page, which is where the actual download lives.
#[tauri::command]
pub fn open_releases(app: AppHandle) -> Result<()> {
    open(&app, RELEASES)
}

/// Opens the project's page, for somebody who wants to read it rather than install it.
#[tauri::command]
pub fn open_repository(app: AppHandle) -> Result<()> {
    open(&app, REPOSITORY)
}

fn open(app: &AppHandle, url: &str) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| crate::error::Error::Webview(error.to_string()))
}

/// Looks now, because somebody asked.
///
/// **Deliberately ignores a skipped version.** The automatic check stays quiet about one the user
/// turned down; pressing a button marked "check for updates" and being told nothing, while a newer
/// release sits there, would be the app keeping a secret it was just asked about.
///
/// Answers `None` for "nothing newer", which is a real answer and worth saying out loud — a check
/// that reports only good news leaves you wondering whether it ran.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<String>> {
    let Some(newest) = fetch().await else {
        return Err(crate::error::Error::Unreachable(
            "Could not reach GitHub to check for updates".into(),
        ));
    };

    let running = app.package_info().version.to_string();

    if !is_newer(&newest, &running) {
        return Ok(None);
    }

    app.state::<Available>().set(Some(newest.clone()));

    Ok(Some(newest))
}

/// Starts the check.
pub fn start(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;

        look(&app).await;
    });
}

/// One look. Once per launch: a phone is opened often enough that this is plenty, and a timer
/// running all day to re-ask a question whose answer changes monthly is not worth the wakeups.
async fn look(app: &AppHandle) {
    let newest = match fetch().await {
        Some(version) => version,
        // offline, or no manifest on the newest release. Neither is news.
        None => return,
    };

    let running = app.package_info().version.to_string();

    if !is_newer(&newest, &running) {
        return;
    }

    // The user has seen this one and said no. Not "no updates" — a release after it is news again.
    if app
        .state::<crate::store::Store>()
        .registry()
        .settings
        .skipped_update
        .as_deref()
        == Some(newest.as_str())
    {
        return;
    }

    eprintln!("[shiver] {newest} is available, running {running}");

    app.state::<Available>().set(Some(newest.clone()));

    use tauri_plugin_notification::NotificationExt;

    if let Err(error) = app
        .notification()
        .builder()
        .id(NOTIFICATION_ID)
        .title(format!("Shiver {newest} is available"))
        .body("Tap to open the releases page.")
        .show()
    {
        // the settings screen still has it, so this is a lost convenience rather than a lost fact
        eprintln!("[shiver] could not announce the update: {error}");
    }
}

async fn fetch() -> Option<String> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .ok()?
        .get(MANIFEST)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        // A release without `latest.json` answers 404 here. Worth a line, because the failure is
        // otherwise perfectly silent and looks exactly like being up to date.
        eprintln!("[shiver] no update manifest: {}", response.status());

        return None;
    }

    response.json::<Manifest>().await.ok().map(|m| m.version)
}

/// Whether `newest` is actually newer, compared the way versions mean rather than the way strings
/// sort — "0.1.10" is above "0.1.9", which no string comparison will tell you.
fn is_newer(newest: &str, running: &str) -> bool {
    match (
        semver::Version::parse(newest),
        semver::Version::parse(running),
    ) {
        (Ok(newest), Ok(running)) => newest > running,
        // an unparseable version is not grounds for telling somebody to go and download something
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn a_higher_version_is_newer() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.2.0", "0.1.9"));
    }

    #[test]
    fn string_order_is_not_version_order() {
        // the case that makes this function exist rather than a `>` on two strings
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.9", "0.1.10"));
    }

    #[test]
    fn the_same_version_is_not_newer() {
        assert!(!is_newer("0.1.1", "0.1.1"));
    }

    #[test]
    fn an_older_version_is_not_newer() {
        assert!(!is_newer("0.1.0", "0.1.1"));
    }

    #[test]
    fn a_prerelease_sits_below_its_release() {
        // the shape of the desktop test build: 0.1.0-b must see 0.1.0 as newer
        assert!(is_newer("0.1.0", "0.1.0-b"));
        assert!(!is_newer("0.1.0-b", "0.1.0"));
    }

    #[test]
    fn nonsense_is_never_newer() {
        assert!(!is_newer("not a version", "0.1.0"));
        assert!(!is_newer("0.1.1", "also not a version"));
    }
}

/// Turns one version down, so nothing mentions it again.
///
/// Not on the next launch either, which is the point: being asked twice about the same thing is how
/// a prompt teaches people to dismiss prompts.
#[tauri::command]
pub fn skip_update(app: AppHandle, version: String) -> Result<()> {
    app.state::<crate::store::Store>().update(|registry| {
        registry.settings.skipped_update = Some(version.clone());

        Ok(())
    })?;

    app.state::<Available>().set(None);

    Ok(())
}
