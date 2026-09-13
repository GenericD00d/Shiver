//! The global mute shortcut.
//!
//! Muting the microphone is the one voice control worth reaching without Shiver in front of you: the
//! rail's own button needs the window, and the window is exactly what is not there when someone
//! starts talking about you while you are in another application. So this one is registered with
//! the operating system rather than the page.
//!
//! It acts on whichever server holds the call, the same as the rail's button, and does nothing at
//! all when there is no call — a shortcut that silently toggles state nobody can see would be worse
//! than one that does nothing.

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{voice::VoiceState, webviews};

/// Applies the user's shortcut, replacing whatever was registered before.
///
/// Everything is unregistered first rather than tracking what was there: the alternative is Shiver
/// believing it holds a shortcut the OS has since taken away, and then quietly holding two.
pub fn apply(app: &AppHandle, accelerator: Option<&str>) {
    let shortcuts = app.global_shortcut();

    if let Err(error) = shortcuts.unregister_all() {
        eprintln!("[shiver] could not clear the old mute shortcut: {error}");
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
        // only the press: a shortcut that fired on release as well would toggle twice per tap
        if event.state() != ShortcutState::Pressed {
            return;
        }

        toggle_mic(&handle);
    });

    if let Err(error) = registered {
        // the usual cause is another application already holding the combination, which is the
        // user's to resolve and worth saying plainly
        eprintln!("[shiver] could not register '{accelerator}' as the mute shortcut: {error}");
    }
}

fn toggle_mic(app: &AppHandle) {
    let Some(holder) = app.state::<VoiceState>().holder() else {
        return;
    };

    webviews::run_voice_action(app, &holder, "mic");
}
