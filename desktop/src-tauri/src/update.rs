//! Noticing that a newer Shiver exists, and installing it when asked.
//!
//! The check is a GET of a manifest published as a release asset; the install is a download whose
//! **minisign signature must verify against a public key compiled into this binary** before a single
//! byte of it is executed. That last part is the point of the whole feature. Shiver's installers are
//! unsigned as far as Windows is concerned, and an updater that merely fetched and ran one would
//! have turned "somebody could publish a bad release" from a thing each user has to be talked into
//! running into a thing every user's machine does by itself. With the signature check, losing
//! control of the GitHub account is not enough to ship code to anyone: the private key is not there.
//!
//! What the user sees is one entry in the notification feed, because that is where Shiver already
//! puts things it wants noticed, and the count on the bell already means "something is waiting".
//!
//! **Desktop only.** Tauri's updater has no Android implementation, and Android would not accept a
//! self-installed package without `REQUEST_INSTALL_PACKAGES` — a permission worth more scrutiny than
//! this feature is worth. The phone finds out the same way it always did.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::error::{Error, Result};
use crate::feed::Feed;
use crate::store::{RegistryStore, Store};

/// How long after launch the first check happens.
///
/// Not immediately: start-up is already busy connecting to every server, and an update that has
/// been available for a week can wait another half minute.
const FIRST_CHECK: Duration = Duration::from_secs(30);

/// The project itself, for the link in About.
const REPOSITORY: &str = "https://github.com/GenericD00d/Shiver";

/// How often to look afterwards. Rarely, deliberately — this is a release channel, not a feed.
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// The newer version, once one is known and the user has not turned it down.
///
/// Read by the shell on launch, which is what puts the prompt in front of somebody rather than
/// leaving it in the feed for them to find. Kept rather than re-fetched: a window opens far more
/// often than a release happens.
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

/// The version already announced, so a check every six hours does not add an entry every six hours.
#[derive(Default)]
pub struct Announced(Mutex<Option<String>>);

impl Announced {
    fn already(&self, version: &str) -> bool {
        let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());

        if held.as_deref() == Some(version) {
            return true;
        }

        *held = Some(version.to_string());

        false
    }
}

/// Starts the background check.
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

/// One check. Quiet about failure: being offline is the ordinary case, not news.
async fn look(app: &AppHandle) {
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            eprintln!("[shiver] the updater is unavailable: {error}");

            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();

            // The user has seen this one and said no. Not "no updates" — a release after it is news
            // again — just not this one, and not every six hours for as long as Shiver is open.
            if app
                .state::<Store>()
                .registry()
                .settings
                .skipped_update
                .as_deref()
                == Some(&version)
            {
                return;
            }

            if app.state::<Announced>().already(&version) {
                return;
            }

            eprintln!("[shiver] {version} is available");

            app.state::<Available>().set(Some(version.clone()));
            app.state::<Feed>().push_update(&version);
            crate::drain::notify_feed_changed(app);
        }
        // up to date, which is the answer most of the time
        Ok(None) => {}
        Err(error) => eprintln!("[shiver] could not check for an update: {error}"),
    }
}

/// Downloads the update and installs it, which ends this process.
///
/// Checked again rather than held from the earlier look: an `Update` is not worth keeping alive for
/// six hours to save one request, and re-checking means the thing installed is the thing that is
/// current at the moment the user asked for it.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<()> {
    let updater = app
        .updater()
        .map_err(|error| Error::Webview(format!("The updater is unavailable: {error}")))?;

    let update = updater
        .check()
        .await
        .map_err(|error| Error::Webview(format!("Could not reach the update server: {error}")))?
        .ok_or_else(|| Error::Webview("Shiver is already up to date".into()))?;

    eprintln!("[shiver] downloading {}", update.version);

    // The signature is checked inside this call, against the key built into this binary. A download
    // that does not verify never reaches the disk as something runnable.
    update
        .download_and_install(|_, _| {}, || eprintln!("[shiver] downloaded, handing over"))
        .await
        .map_err(|error| Error::Webview(format!("The update could not be installed: {error}")))?;

    // On Windows the installer takes over from here and this process is replaced. Asking to exit is
    // what lets it: an installer cannot overwrite a running binary.
    app.exit(0);

    Ok(())
}

/// What the shell asks on launch, to decide whether to say anything.
#[tauri::command]
pub fn available_update(app: AppHandle) -> Option<String> {
    app.state::<Available>().get()
}

/// Turns one version down.
///
/// The prompt does not come back for it — not on the next check, and not on the next launch, which
/// is the point: being asked twice about the same thing is how a prompt teaches people to dismiss
/// prompts. The entry in the feed goes too, so nothing is left claiming there is something to do.
#[tauri::command]
pub fn skip_update(app: AppHandle, store: tauri::State<'_, Store>, version: String) -> Result<()> {
    store.update(|registry| {
        registry.settings.skipped_update = Some(version.clone());

        Ok(())
    })?;

    app.state::<Available>().set(None);
    app.state::<Feed>().forget_updates();
    crate::drain::notify_feed_changed(&app);

    Ok(())
}

/// Opens the project's page, for somebody who wants to read it rather than install it.
#[tauri::command]
pub fn open_repository(app: AppHandle) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_url(REPOSITORY, None::<&str>)
        .map_err(|error| Error::Webview(error.to_string()))
}

/// Looks now, because somebody asked.
///
/// **Deliberately ignores a skipped version.** The background check stays quiet about one the user
/// turned down; pressing a button marked "check for updates" and being told nothing, while a newer
/// release sits there, would be the app keeping a secret it was just asked about.
///
/// Answers `None` for "nothing newer", which is a real answer and worth saying out loud — a check
/// that reports only good news leaves you wondering whether it ran.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<String>> {
    let updater = app
        .updater()
        .map_err(|error| Error::Webview(format!("The updater is unavailable: {error}")))?;

    let found = updater
        .check()
        .await
        .map_err(|error| Error::Webview(format!("Could not reach the update server: {error}")))?;

    let Some(update) = found else {
        return Ok(None);
    };

    let version = update.version.clone();

    app.state::<Available>().set(Some(version.clone()));

    Ok(Some(version))
}
