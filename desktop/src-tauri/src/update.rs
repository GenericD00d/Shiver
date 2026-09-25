//! Checks for a newer Shiver and installs it when asked. The download's minisign signature must
//! verify against the public key compiled into this binary (`tauri.conf.json`) before it runs.
//! A new version shows up as one entry in the notification feed.

use std::sync::Mutex;
use std::time::Duration;

use shiver_core::LockExt;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{
    error::{Error, Result},
    feed::Feed,
    store::{RegistryStore, Store},
};

const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const REPOSITORY: &str = "https://github.com/GenericD00d/Shiver";

/// Tells the shell a newer version was found, so its bar appears without asking on a timer.
const UPDATE_EVENT: &str = "shiver://update";

/// The newer version, once known and not turned down.
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

/// The updater's answer now: the newer release, if there is one.
async fn check(app: &AppHandle) -> Result<Option<Update>> {
    app.updater()
        .map_err(|error| Error::Webview(format!("The updater is unavailable: {error}")))?
        .check()
        .await
        .map_err(|error| Error::Webview(format!("Could not reach the update server: {error}")))
}

/// Starts the background check: once after `FIRST_CHECK`, then every `EVERY`.
pub fn start(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;

        loop {
            look(&app).await;
            tokio::time::sleep(EVERY).await;
        }
    });
}

/// One background check. Quiet about failure: being offline is ordinary.
async fn look(app: &AppHandle) {
    let version = match check(app).await {
        Ok(Some(update)) => update.version,
        Ok(None) => return,
        Err(error) => return eprintln!("[shiver] {error}"),
    };

    let skipped = app
        .state::<Store>()
        .registry()
        .settings
        .skipped_update
        .as_deref()
        == Some(version.as_str());

    // turned down, or already offered
    if skipped || app.state::<Available>().get().as_deref() == Some(version.as_str()) {
        return;
    }

    app.state::<Available>().set(Some(version.clone()));
    app.state::<Feed>().push_update(&version);
    crate::drain::notify_feed_changed(app);
    let _ = app.emit_to(crate::webviews::SHELL_WEBVIEW, UPDATE_EVENT, &version);
}

/// Downloads, verifies and installs the current release, then exits so the installer can replace
/// the binary.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<()> {
    let update = check(&app)
        .await?
        .ok_or_else(|| Error::Webview("Shiver is already up to date".into()))?;

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| Error::Webview(format!("The update could not be installed: {error}")))?;

    app.exit(0);

    Ok(())
}

#[tauri::command]
pub fn available_update(app: AppHandle) -> Option<String> {
    app.state::<Available>().get()
}

/// Turns one version down for good; a later release is offered again.
#[tauri::command]
pub async fn skip_update(app: AppHandle, version: String) -> Result<()> {
    app.state::<Store>().update(|registry| {
        registry.settings.skipped_update = Some(version);

        Ok(())
    })?;

    app.state::<Available>().set(None);
    app.state::<Feed>().forget_updates();
    crate::drain::notify_feed_changed(&app);

    Ok(())
}

#[tauri::command]
pub fn open_repository(app: AppHandle) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_url(REPOSITORY, None::<&str>)
        .map_err(|error| Error::Webview(error.to_string()))
}

/// Checks now, on request. Reports a skipped version too, since the user asked.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<String>> {
    let version = check(&app).await?.map(|update| update.version);

    if version.is_some() {
        app.state::<Available>().set(version.clone());
    }

    Ok(version)
}
