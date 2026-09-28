//! The tray icon, closing to it, and starting with the system.
//!
//! The icon is up only while "close to the tray" is on: closing the window then hides it (servers
//! stay connected, and notifications keep arriving), and the icon brings it back or quits. Started
//! at login (the login item passes `AT_LOGIN_ARG`), Shiver opens minimised, or straight to the
//! tray when it closes there.

use std::sync::atomic::{AtomicUsize, Ordering};

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};
use tauri_plugin_autostart::ManagerExt;

use crate::{store::Store, webviews};

const TRAY_ID: &str = "shiver";
// no `:` in these: the app's own menu handler reads `action:entry` ids as a server's menu
const SHOW_ID: &str = "tray-show";
const QUIT_ID: &str = "tray-quit";

/// What the login item starts Shiver with.
pub const AT_LOGIN_ARG: &str = "--at-login";

/// Puts the tray icon up, or takes it down, to match the setting.
pub fn apply(app: &AppHandle, close_to_tray: bool) {
    let shown = app.tray_by_id(TRAY_ID).is_some();

    if close_to_tray && !shown {
        if let Err(error) = build(app) {
            // without an icon a hidden window could not come back, so closing quits as before
            eprintln!("[shiver] could not put up the tray icon: {error}");
        }
    } else if !close_to_tray && shown {
        let _ = app.remove_tray_by_id(TRAY_ID);
    }
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id(SHOW_ID, "Show Shiver").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id(QUIT_ID, "Quit Shiver").build(app)?)
        .build()?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Shiver")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_ID => show(app),
            QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;

    Ok(())
}

/// Brings the window back from the tray, the taskbar or behind other windows.
pub fn show(app: &AppHandle) {
    let Ok(window) = webviews::main_window(app) else {
        return;
    };

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    webviews::set_window_hidden(app, false);
}

/// Hides the window instead of closing it, while the tray icon is up to bring it back. Returns
/// whether it did (so the close should be prevented).
pub fn hide_instead_of_closing(app: &AppHandle) -> bool {
    if app.tray_by_id(TRAY_ID).is_none() {
        return false;
    }

    if let Ok(window) = webviews::main_window(app) {
        let _ = window.hide();
    }

    webviews::set_window_hidden(app, true);

    true
}

/// Started by the login item: out of the way, in the tray when there is one, else minimised.
pub fn start_out_of_the_way(app: &AppHandle) {
    if !std::env::args().any(|arg| arg == AT_LOGIN_ARG) || hide_instead_of_closing(app) {
        return;
    }

    if let Ok(window) = webviews::main_window(app) {
        let _ = window.minimize();
    }
}

/// Registers Shiver to start at login, or stops it, as the setting says.
pub fn set_start_at_login(app: &AppHandle, start: bool) {
    let launcher = app.autolaunch();

    if launcher.is_enabled().unwrap_or(!start) == start {
        return;
    }

    let result = if start {
        launcher.enable()
    } else {
        launcher.disable()
    };

    if let Err(error) = result {
        eprintln!("[shiver] could not change starting at login: {error}");
    }
}

/// Keeps the icon's tooltip on the same total the taskbar badge shows (set only when it changes).
pub fn show_unread<R: Runtime>(app: &AppHandle<R>, unread: usize) {
    static SHOWN: AtomicUsize = AtomicUsize::new(usize::MAX);

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        // a new icon starts at "Shiver", so the next count is set on it
        SHOWN.store(usize::MAX, Ordering::Relaxed);

        return;
    };

    if SHOWN.swap(unread, Ordering::Relaxed) == unread {
        return;
    }

    let tooltip = match unread {
        0 => "Shiver".to_string(),
        count => format!("Shiver: {count} unread"),
    };

    let _ = tray.set_tooltip(Some(tooltip));
}

/// Applies both settings at startup, then gets out of the way if the system started Shiver.
pub fn start(app: &AppHandle) {
    let (close_to_tray, start_at_login) = {
        let store = app.state::<Store>();
        let settings = &store.registry().settings;

        (settings.close_to_tray, settings.start_at_login)
    };

    apply(app, close_to_tray);

    // only ever turned on here: an entry the user removed by hand stays removed until they ask
    if start_at_login {
        set_start_at_login(app, true);
    }

    start_out_of_the_way(app);
}
