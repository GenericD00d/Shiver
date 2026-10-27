//! `sharkord://` links: Sharkord's sign-in page offers a desktop app `sharkord://add-server?url=…`,
//! which opens Shiver's add screen with that server's address filled in (or the server itself, when
//! it is already in the rail). The link is only a suggestion: the user still checks the server and
//! signs in. Installers register the scheme (`plugins.deep-link` in tauri.conf.json); a Linux
//! AppImage, which no installer sets up, registers it at startup, as does a debug build.
//!
//! A link that starts Shiver arrives before the shell can listen, so the latest one waits here
//! until the shell asks (`take_server_link`), on load and on each `shiver://server-link`.

use std::sync::Mutex;

use shiver_core::LockExt;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use crate::{tray, webviews};

const SERVER_LINK_EVENT: &str = "shiver://server-link";

/// The origin the latest link offered, until the shell takes it.
#[derive(Default)]
pub struct PendingLink(Mutex<Option<String>>);

/// Takes the links Shiver was started with, and those that reach it while it runs.
pub fn start(app: &AppHandle) {
    #[cfg(any(target_os = "linux", all(windows, debug_assertions)))]
    if let Err(error) = app.deep_link().register_all() {
        eprintln!("[shiver] could not register sharkord:// links: {error}");
    }

    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in urls {
            offer(app, url.as_str());
        }
    }

    let handle = app.clone();

    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            offer(&handle, url.as_str());
        }
    });
}

/// Keeps the server a link offers for the shell, and brings Shiver to the front for it.
fn offer(app: &AppHandle, link: &str) {
    let Some(origin) = shiver_core::origin::add_server_link(link) else {
        return;
    };

    *app.state::<PendingLink>().0.locked() = Some(origin);
    tray::show(app);
    let _ = app.emit_to(webviews::SHELL_WEBVIEW, SERVER_LINK_EVENT, ());
}

/// The server the latest link offered, once (the shell opens it, or its add screen).
#[tauri::command]
pub fn take_server_link(app: AppHandle) -> Option<String> {
    app.state::<PendingLink>().0.locked().take()
}
