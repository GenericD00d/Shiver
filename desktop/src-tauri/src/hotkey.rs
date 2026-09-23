//! The global microphone mute shortcut, acting on whichever server holds the call.

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{voice::VoiceState, webviews};

/// Replaces the registered shortcut with `accelerator` (or none).
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
        if event.state() != ShortcutState::Pressed {
            return;
        }

        if let Some(holder) = handle.state::<VoiceState>().holder() {
            webviews::run_voice_action(&handle, &holder, "mic");
        }
    });

    if let Err(error) = registered {
        // usually another application holds the combination
        eprintln!("[shiver] could not register '{accelerator}' as the mute shortcut: {error}");
    }
}
