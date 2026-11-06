//! The tray icon, closing to it, and starting with the system.
//!
//! The icon is up only while "close to the tray" is on: closing the window then hides it (servers
//! stay connected, and notifications keep arriving), and the icon brings it back or quits. While
//! anything is unread it carries a dot, as Sharkord's desktop app's does, so a Shiver closed to
//! the tray still shows it. Started
//! at login (the login item passes `AT_LOGIN_ARG`), Shiver opens minimised, or straight to the
//! tray when it closes there.

use std::sync::atomic::{AtomicUsize, Ordering};

use tauri::{
    image::Image,
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
            log::warn!("could not put up the tray icon: {error}");
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
        log::warn!("could not change starting at login: {error}");
    }
}

/// Keeps the icon's tooltip on the same total the taskbar badge shows, and its dot on whether there
/// is any (each set only when it changes).
pub fn show_unread<R: Runtime>(app: &AppHandle<R>, unread: usize) {
    static SHOWN: AtomicUsize = AtomicUsize::new(usize::MAX);

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        // a new icon starts at "Shiver", so the next count is set on it
        SHOWN.store(usize::MAX, Ordering::Relaxed);

        return;
    };

    let shown = SHOWN.swap(unread, Ordering::Relaxed);

    if shown == unread {
        return;
    }

    let tooltip = match unread {
        0 => "Shiver".to_string(),
        count => format!("Shiver: {count} unread"),
    };

    let _ = tray.set_tooltip(Some(tooltip));

    // a new icon starts without the dot
    let had_dot = shown != usize::MAX && shown > 0;

    if had_dot != (unread > 0) {
        if let Some(icon) = app.default_window_icon() {
            let icon = if unread > 0 {
                with_dot(icon)
            } else {
                icon.clone()
            };

            let _ = tray.set_icon(Some(icon));
        }
    }
}

/// The icon with an unread dot in its lower right corner, ringed in near-black so it stands apart
/// from the icon under it, at any size the icon comes in.
fn with_dot(icon: &Image<'_>) -> Image<'static> {
    let (width, height) = (icon.width(), icon.height());
    let size = width.min(height) as f32;
    let radius = size * 0.24;
    let ring = (size * 0.06).max(1.0);
    let centre = (width as f32 - radius - 0.5, height as f32 - radius - 0.5);
    let mut rgba = icon.rgba().to_vec();

    for y in 0..height {
        for x in 0..width {
            let distance = (x as f32 - centre.0).hypot(y as f32 - centre.1);
            let coverage = (radius - distance).clamp(0.0, 1.0);

            if coverage == 0.0 {
                continue;
            }

            let colour = if distance > radius - ring {
                OUTLINE
            } else {
                crate::badge::DOT
            };
            let pixel = &mut rgba[((y * width + x) * 4) as usize..][..4];

            for channel in 0..3 {
                pixel[channel] = (colour[channel] as f32 * coverage
                    + pixel[channel] as f32 * (1.0 - coverage))
                    as u8;
            }

            pixel[3] = (255.0 * coverage + pixel[3] as f32 * (1.0 - coverage)) as u8;
        }
    }

    Image::new_owned(rgba, width, height)
}

/// The ring around the unread dot.
const OUTLINE: [u8; 3] = [0x1a, 0x1a, 0x1a];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_sits_in_the_lower_right_and_leaves_the_rest_of_the_icon() {
        let icon = Image::new_owned(vec![0x40; 32 * 32 * 4], 32, 32);
        let dotted = with_dot(&icon);
        let pixel = |x: u32, y: u32| dotted.rgba()[((y * 32 + x) * 4) as usize..][..4].to_vec();
        let centre = (32.0 - 32.0 * 0.24 - 0.5) as u32;

        assert_eq!(pixel(2, 2), vec![0x40; 4]);
        assert_eq!(
            pixel(centre, centre),
            vec![
                crate::badge::DOT[0],
                crate::badge::DOT[1],
                crate::badge::DOT[2],
                255
            ]
        );
    }
}
