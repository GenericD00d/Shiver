//! The quick rail: the servers, drawn natively over the server page on screen
//! (`tauri-plugin-shiver-rail`), so reaching them no longer unloads the page. Coming back to the
//! same server is closing the rail; the page never stopped, so it never joins the server again.
//!
//! A server's page asks for the rail the only way it can, by navigating to Shiver's page with
//! `#home` (a swipe past the drawer, or back); that navigation is cancelled and the rail drawn
//! instead. Choosing another server, or Shiver's own page, leaves through Shiver's page as before.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use shiver_core::{
    model::palette,
    rail::{initials, rows, RailRow},
    LockExt,
};
use tauri::{AppHandle, Manager, Url};
use tauri_plugin_shiver_rail::{RailEvent, RailExt, RailView, Row, Tile};

use crate::{icons, inbox::Inbox, model::ServerEntry, store::Store, webview};

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

/// What the user chose. The rail has already closed itself.
pub fn chosen(app: &AppHandle, event: RailEvent) {
    app.state::<QuickRail>()
        .open
        .store(false, Ordering::Release);

    match event {
        RailEvent::Closed => {}
        RailEvent::Home => webview::go_home(app, Some("home")),
        RailEvent::Open { entry_id } => {
            let known = app.state::<Store>().registry().server(&entry_id).is_some();

            // opened by Shiver's page, which waits for the server to take another join if need be
            if known {
                webview::go_home(app, Some(&format!("open={entry_id}")));
            }
        }
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

            // the swipe still reaches the servers, the old way
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
    let colors = palette(
        &settings.theme_color,
        &settings.accent_color,
        settings.text_color.as_deref(),
    );
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
            initials: initials(&server.name),
            unread: unread.get(&server.id).copied().unwrap_or(0),
            icon_key,
            icon,
        }
    };

    RailView {
        rail: colors.rail,
        surface: colors.surface,
        surface_dim: colors.surface_dim,
        text: colors.text,
        accent: colors.accent,
        current: app.state::<webview::Showing>().server(),
        rows: rows(&servers, &folders)
            .into_iter()
            .map(|row| match row {
                RailRow::Server(server) => Row::Server {
                    server: tile(server),
                },
                RailRow::Folder(folder, members) => Row::Folder {
                    name: folder.name.clone(),
                    servers: members.into_iter().map(tile).collect(),
                },
            })
            .collect(),
    }
}
