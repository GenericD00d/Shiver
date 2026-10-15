//! The quick rail over the server page (`tauri-plugin-shiver-rail`), so reaching Shiver's screens
//! does not unload it; coming back to the same server only closes the rail.
//!
//! A server's page asks for it by navigating to Shiver's page with `#home` (a swipe past the drawer,
//! or back); that navigation is cancelled and the rail drawn instead. Another server, or the screen
//! for adding one, is reached through Shiver's page (`#open=<id>`, `#add`). A tile's menu, the
//! settings and the direct messages are drawn over the page by the rail's (`overlay`), and a
//! conversation on the server behind opens in its page.

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
    /// asked for and not since taken down: a `close` while it is still being drawn clears this, and
    /// the draw then takes it down again rather than leaving it over the new page
    wanted: AtomicBool,
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
    app.state::<QuickRail>()
        .wanted
        .store(true, Ordering::Release);

    let app = app.clone();

    std::thread::spawn(move || draw(&app, true));
}

/// Whether the rail (or a screen it opened) is over the page.
pub fn is_open(app: &AppHandle) -> bool {
    app.state::<QuickRail>().open.load(Ordering::Acquire)
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
    let rail = app.state::<QuickRail>();

    rail.wanted.store(false, Ordering::Release);

    if !rail.open.swap(false, Ordering::AcqRel) {
        return;
    }

    let app = app.clone();

    std::thread::spawn(move || {
        let _ = app.shiver_rail().hide();
    });
}

/// What the user chose. The rail has already closed itself, unless a folder was opened or shut or
/// its screens called the core.
pub fn chosen(app: &AppHandle, event: RailEvent) {
    let screen = match event {
        RailEvent::Call { id, cmd, args } => return crate::overlay::call(app, id, cmd, args),
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
        RailEvent::Add => Some("add".to_string()),
        // opened by Shiver's page, which waits for the server to take another join if need be; a
        // conversation on the server behind opens in its page instead
        RailEvent::Open { entry_id, dm_user } => {
            let entry = app.state::<Store>().registry().server(&entry_id).cloned();

            match (entry, dm_user) {
                (Some(entry), Some(user)) if webview::open_dm_in_page(app, &entry, &user) => None,
                (Some(_), user) => {
                    app.state::<webview::Showing>().queue_dm(&entry_id, user);

                    Some(format!("open={entry_id}"))
                }
                (None, _) => None,
            }
        }
    };

    let quick = app.state::<QuickRail>();

    quick.wanted.store(false, Ordering::Release);
    quick.open.store(false, Ordering::Release);

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
        let quick = app.state::<QuickRail>();

        quick.open.store(true, Ordering::Release);

        // closed while it was being drawn: a page started loading under it
        if !quick.wanted.load(Ordering::Acquire) && quick.open.swap(false, Ordering::AcqRel) {
            let _ = rail.hide();

            return;
        }
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
