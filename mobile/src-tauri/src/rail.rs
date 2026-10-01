//! The quick rail over the server page (`tauri-plugin-shiver-rail`), so reaching Shiver's screens
//! does not unload it; coming back to the same server only closes the rail.
//!
//! A server's page asks for it by navigating to Shiver's page with `#home` (a swipe past the drawer,
//! or back); that navigation is cancelled and the rail drawn instead. Another server, or one of
//! Shiver's screens, is reached through Shiver's page (`#open=<id>`, `#dms`, `#add`, `#settings`).

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use shiver_core::LockExt;
use tauri::{AppHandle, Manager, Url};
use tauri_plugin_shiver_rail::{RailEvent, RailExt, RailFolder, RailView, Tile};

use crate::{
    icons,
    inbox::Inbox,
    model::ServerEntry,
    store::{RegistryStore, Store},
    webview,
};

#[derive(Default)]
pub struct QuickRail {
    open: AtomicBool,
    /// the logo key the rail has been given for each entry
    sent: Mutex<HashMap<String, String>>,
}

/// Whether a navigation is a server page asking for the rail.
pub fn wanted(app: &AppHandle, target: &Url) -> bool {
    target.fragment() == Some("home")
        && webview::is_home(app, target)
        && app.state::<webview::Showing>().server().is_some()
}

/// Slides the rail in over the page. Off the calling thread: this is asked from the webview's own
/// (UI) thread, which the rail needs to draw on.
pub fn open(app: &AppHandle) {
    let app = app.clone();

    std::thread::spawn(move || draw(&app, true));
}

/// Redraws the rail if it is up (new unread counts).
pub fn refresh(app: &AppHandle) {
    if !app.state::<QuickRail>().open.load(Ordering::Acquire) {
        return;
    }

    let app = app.clone();

    std::thread::spawn(move || draw(&app, false));
}

/// Takes the rail down, as a page starts loading under it.
pub fn close(app: &AppHandle) {
    if !app.state::<QuickRail>().open.swap(false, Ordering::AcqRel) {
        return;
    }

    let app = app.clone();

    std::thread::spawn(move || {
        let _ = app.shiver_rail().hide();
    });
}

/// What the user chose. The rail has already closed itself, unless a folder was opened or shut.
pub fn chosen(app: &AppHandle, event: RailEvent) {
    let screen = match event {
        RailEvent::Folder {
            folder_id,
            expanded,
        } => {
            // an unknown folder stores nothing; either way the rail redraws to match the store
            let _ = app.state::<Store>().update(|registry| {
                Ok(registry.rail().set_folder_expanded(&folder_id, expanded)?)
            });

            return refresh(app);
        }
        RailEvent::Closed => None,
        RailEvent::Dms => Some("dms".to_string()),
        RailEvent::Add => Some("add".to_string()),
        RailEvent::Settings => Some("settings".to_string()),
        // opened by Shiver's page, which waits for the server to take another join if need be
        RailEvent::Open { entry_id } => app
            .state::<Store>()
            .registry()
            .server(&entry_id)
            .map(|_| format!("open={entry_id}")),
    };

    app.state::<QuickRail>()
        .open
        .store(false, Ordering::Release);

    if let Some(fragment) = screen {
        webview::go_home(app, Some(&fragment));
    }
}

fn draw(app: &AppHandle, reveal: bool) {
    let rail = app.shiver_rail();
    let view = view(app);
    let drawn = if reveal {
        rail.show(&view)
    } else {
        rail.refresh(&view)
    };

    let missing = match drawn {
        Ok(missing) => missing,
        Err(error) => {
            eprintln!("[shiver] could not draw the rail: {error}");

            // the swipe still reaches the servers, the old way: Shiver's page, with its own rail
            if reveal {
                webview::go_home(app, Some("home"));
            }

            return;
        }
    };

    if reveal {
        app.state::<QuickRail>().open.store(true, Ordering::Release);
    }

    // the rail lost logos it was given (a new activity): sent again
    if !missing.is_empty() {
        {
            let rail_state = app.state::<QuickRail>();
            let mut sent = rail_state.sent.locked();

            for id in &missing {
                sent.remove(id);
            }
        }

        let _ = rail.refresh(&self::view(app));
    }
}

fn view(app: &AppHandle) -> RailView {
    let (servers, folders, settings) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        (
            registry.servers.clone(),
            registry.folders.clone(),
            registry.settings.clone(),
        )
    };
    let unread = app.state::<Inbox>().unread();
    let tile = |server: &ServerEntry| {
        let icon_key = icons::key(app, &server.id);
        let icon = icon_key.as_ref().and_then(|key| {
            let rail_state = app.state::<QuickRail>();
            let mut sent = rail_state.sent.locked();

            if sent.get(&server.id) == Some(key) {
                return None;
            }

            let data = icons::load_one(app, &server.id)?;

            sent.insert(server.id.clone(), key.clone());

            Some(data)
        });

        Tile {
            id: server.id.clone(),
            name: server.name.clone(),
            folder_id: server.folder_id.clone(),
            position: server.position,
            unread: unread.get(&server.id).copied().unwrap_or(0),
            icon_key,
            icon,
        }
    };

    RailView {
        theme_color: settings.theme_color,
        accent_color: settings.accent_color,
        text_color: settings.text_color,
        current: app.state::<webview::Showing>().server(),
        servers: servers.iter().map(tile).collect(),
        folders: folders
            .into_iter()
            .map(|folder| RailFolder {
                id: folder.id,
                name: folder.name,
                position: folder.position,
                expanded: folder.expanded,
            })
            .collect(),
    }
}
