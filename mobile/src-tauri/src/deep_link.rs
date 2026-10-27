//! `sharkord://` links: Sharkord's sign-in page offers a desktop app `sharkord://add-server?url=…`,
//! which on Android opens Shiver (the scheme is in the manifest, from `plugins.deep-link` in
//! tauri.conf.json) on its add screen with the server's address filled in, or on the server itself
//! when it is already in the rail. The link is only a suggestion: the user still checks the server
//! and signs in.
//!
//! The latest link waits here until Shiver's page takes it (`take_server_link`): as it boots, and
//! on `shiver://server-link` while it is up. A link that arrives over a server's page sends the
//! webview home for it, unless that page is the server the link names.

use std::sync::Mutex;

use shiver_core::LockExt;
use tauri::{AppHandle, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use crate::{
    store::Store,
    webview::{self, Showing},
};

const SERVER_LINK_EVENT: &str = "shiver://server-link";

/// The origin the latest link offered, until Shiver's page takes it.
#[derive(Default)]
pub struct PendingLink(Mutex<Option<String>>);

/// Takes the link Shiver was opened with, and those that reach it while it runs.
pub fn start(app: &AppHandle) {
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

fn offer(app: &AppHandle, link: &str) {
    let Some(origin) = shiver_core::origin::add_server_link(link) else {
        return;
    };

    let showing = app.state::<Showing>();
    let named = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .find(|server| server.origin == origin)
        .map(|server| server.id.clone());

    // already where the link leads
    if named.is_some() && showing.server() == named {
        return;
    }

    *app.state::<PendingLink>().0.locked() = Some(origin);

    if showing.at_home() {
        webview::emit_home(app, SERVER_LINK_EVENT, ());
    } else {
        // Shiver's page takes it as it boots
        webview::go_home(app, None);
    }
}

/// The server the latest link offered, once (Shiver's page opens it, or its add screen).
#[tauri::command]
pub fn take_server_link(app: AppHandle) -> Option<String> {
    app.state::<PendingLink>().0.locked().take()
}
