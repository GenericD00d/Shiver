//! Shiver's log file, as Sharkord's desktop app keeps one: what the core reports (the `log` crate,
//! shared crates included), a panic, and the errors each server's page throws (`page_errors`), in
//! `shiver.log` in the system's log folder for the app. Capped at `MAX_FILE_SIZE`, with the one
//! before it kept beside it, so a long session still holds its start. Attach it to a bug report.

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};
use tauri_plugin_opener::OpenerExt;

use crate::error::{Error, Result};

const FILE_NAME: &str = "shiver";
const MAX_FILE_SIZE: u128 = 5 * 1024 * 1024;
/// The log target a page's errors are written under.
const PAGE_TARGET: &str = "page";
/// Most errors one drain may write for a page (the bridge holds 30 and a count of the rest), and
/// most characters of each.
const MAX_PAGE_ERRORS: usize = 31;
const MAX_PAGE_ERROR: usize = 1000;

/// The logger: Shiver's own reports from info up, everyone else's (Tauri, WebView2, the network)
/// from warnings up, to the file and to stdout.
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::LogDir {
                file_name: Some(FILE_NAME.into()),
            }),
        ])
        .level(log::LevelFilter::Warn)
        .level_for("shiver_lib", log::LevelFilter::Info)
        .level_for("shiver_core", log::LevelFilter::Info)
        .level_for("sharkord_client", log::LevelFilter::Info)
        .level_for(PAGE_TARGET, log::LevelFilter::Info)
        .max_file_size(MAX_FILE_SIZE)
        .rotation_strategy(RotationStrategy::KeepSome(2))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .build()
}

/// The first lines of a run: which Shiver, on what, so a log answers the first question about a bug.
/// A panic is written too, before the process aborts.
pub fn start(app: &AppHandle) {
    log::info!(
        "Shiver {} starting on {} {}",
        app.package_info().version,
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        log::logger().flush();
        previous(info);
    }));
}

/// Writes what a server's page threw (uncaught errors, rejections, `console.error`), each line
/// bounded, under the `page` target with the server's entry id.
pub fn page_errors(entry_id: &str, errors: &[String]) {
    for error in errors.iter().take(MAX_PAGE_ERRORS) {
        let error: String = error.chars().take(MAX_PAGE_ERROR).collect();

        log::warn!(target: PAGE_TARGET, "{entry_id}: {error}");
    }
}

/// Opens the folder the log is in, for attaching it to a report.
#[tauri::command]
pub fn open_logs_folder(app: AppHandle) -> Result<()> {
    let folder = app
        .path()
        .app_log_dir()
        .map_err(|error| Error::Webview(format!("Shiver has no log folder: {error}")))?;

    std::fs::create_dir_all(&folder)
        .map_err(|error| Error::Webview(format!("Could not make the log folder: {error}")))?;

    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(|error| Error::Webview(format!("Could not open the log folder: {error}")))
}
