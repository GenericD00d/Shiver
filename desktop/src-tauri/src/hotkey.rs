//! Keyboard shortcuts: the system-wide microphone mute, acting on whichever server holds the call,
//! and the rail's, which exist only while Shiver is in front.
//!
//! The rail's are global shortcuts too, since the keyboard is usually inside a server's webview,
//! where Shiver's own pages hear nothing. Registered for as long as Shiver is in front and no
//! longer (a global shortcut takes its keys from every other application): Ctrl+1…9 (Cmd on macOS)
//! opens the rail's nth server, Ctrl+Alt+Up/Down the one above or below. The shell picks the
//! server, since it knows the rail's order.

use std::{sync::Mutex, time::Duration};

use shiver_core::LockExt;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{voice::VoiceState, webviews};

/// Tells the shell a rail shortcut was pressed: `{"server": n}` (1-based) or `{"step": ±1}`.
const SHORTCUT_EVENT: &str = "shiver://shortcut";

/// How often Shiver checks whether it is in front (the window's own focus events miss a switch made
/// while the keyboard is in a webview).
const FRONT_POLL: Duration = Duration::from_millis(200);

/// The mute shortcut now registered, so replacing it leaves the rail's alone.
static MUTE: Mutex<Option<Shortcut>> = Mutex::new(None);

/// Replaces the mute shortcut with `accelerator` (or none).
pub fn apply(app: &AppHandle, accelerator: Option<&str>) {
    let shortcuts = app.global_shortcut();

    if let Some(previous) = MUTE.locked().take() {
        if let Err(error) = shortcuts.unregister(previous) {
            eprintln!("[shiver] could not clear the old mute shortcut: {error}");
        }
    }

    let Some(accelerator) = accelerator.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };

    let shortcut: Shortcut = match accelerator.parse() {
        Ok(shortcut) => shortcut,
        Err(error) => {
            eprintln!("[shiver] '{accelerator}' is not a shortcut Shiver can register: {error}");

            return;
        }
    };

    let handle = app.clone();

    let registered = shortcuts.on_shortcut(shortcut, move |_app, _shortcut, event| {
        if event.state() != ShortcutState::Pressed {
            return;
        }

        if let Some(holder) = handle.state::<VoiceState>().holder() {
            webviews::run_voice_action(&handle, &holder, "mic");
        }
    });

    match registered {
        Ok(()) => *MUTE.locked() = Some(shortcut),
        // usually another application holds the combination
        Err(error) => {
            eprintln!("[shiver] could not register '{accelerator}' as the mute shortcut: {error}")
        }
    }
}

/// The rail's shortcuts and what each tells the shell.
fn rail_shortcuts() -> Vec<(Shortcut, serde_json::Value)> {
    let numbered = (1..=9).map(|n| {
        (
            format!("CommandOrControl+{n}"),
            serde_json::json!({ "server": n }),
        )
    });
    let stepped = [("ArrowUp", -1), ("ArrowDown", 1)].map(|(key, step)| {
        (
            format!("CommandOrControl+Alt+{key}"),
            serde_json::json!({ "step": step }),
        )
    });

    numbered
        .chain(stepped)
        .filter_map(|(accelerator, payload)| Some((accelerator.parse().ok()?, payload)))
        .collect()
}

/// Registers the rail's shortcuts, or takes them back. One that another application (or the mute
/// shortcut) holds is skipped.
fn set_rail_shortcuts(app: &AppHandle, on: bool) {
    let shortcuts = app.global_shortcut();
    let mute = *MUTE.locked();

    for (shortcut, payload) in rail_shortcuts() {
        if Some(shortcut) == mute {
            continue;
        }

        if !on {
            let _ = shortcuts.unregister(shortcut);

            continue;
        }

        let handle = app.clone();
        let _ = shortcuts.on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                let _ = handle.emit_to(webviews::SHELL_WEBVIEW, SHORTCUT_EVENT, payload.clone());
            }
        });
    }
}

/// Keeps the rail's shortcuts registered exactly while Shiver is in front.
pub fn watch_front(app: &AppHandle) {
    let app = app.clone();

    std::thread::Builder::new()
        .name("shiver-front".into())
        .spawn(move || {
            let mut registered = false;

            loop {
                std::thread::sleep(FRONT_POLL);

                let front = webviews::is_in_front(&app);

                if front != registered {
                    set_rail_shortcuts(&app, front);
                    registered = front;
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|error| eprintln!("[shiver] could not watch for Shiver in front: {error}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_has_eleven_shortcuts() {
        let shortcuts = rail_shortcuts();

        assert_eq!(shortcuts.len(), 11);
        assert_eq!(shortcuts[0].1, serde_json::json!({ "server": 1 }));
        assert_eq!(shortcuts[10].1, serde_json::json!({ "step": 1 }));
    }
}
