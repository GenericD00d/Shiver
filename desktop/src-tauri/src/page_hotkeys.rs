//! Sharkord's own keyboard shortcuts, for a server whose client offers them to a desktop app
//! (Sharkord's desktop api, `desktop/bridge/desktop-api.ts`): each page hands over its bindings, and
//! those of the server last shown are registered system-wide, as Sharkord's desktop app does, so
//! the others leave their keys free for other applications. A press is told to that page, and so
//! is its release (Sharkord's push-to-talk), to the page that had the press even when another
//! server has been shown since.
//!
//! A combination Shiver's own shortcuts use (the mute shortcut, the rail's) is refused as taken,
//! so the two never take keys from each other.

use std::{collections::HashMap, sync::Mutex};

use serde::{Deserialize, Serialize};
use shiver_core::LockExt;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{hotkey, webviews};

/// Most bindings one page may hand over, as Sharkord's desktop app allows.
const MAX_BINDINGS: usize = 100;
const MAX_ACTION_ID: usize = 200;
const MAX_COMBO: usize = 100;

/// One of a page's bindings: Sharkord's action and its combination (`ctrl+shift+KeyM`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub action_id: String,
    pub combo: String,
}

/// Why a binding was not registered, in Sharkord's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingError {
    pub action_id: String,
    pub reason: &'static str,
}

#[derive(Default)]
struct State {
    bindings: HashMap<String, Vec<(String, Shortcut)>>,
    /// the server whose bindings are registered
    active: Option<String>,
    registered: Vec<Shortcut>,
    /// the shortcuts down, with the server and actions each press went to, for its release
    held: HashMap<Shortcut, (String, Vec<String>)>,
}

#[derive(Default)]
pub struct PageHotkeys(Mutex<State>);

/// A combination as Sharkord records it, modifiers first and the key's physical code last, as a
/// shortcut; none for a modifier Sharkord does not write or a key the system cannot register.
pub fn shortcut_for(combo: &str) -> Option<Shortcut> {
    let mut parts: Vec<&str> = combo.split('+').collect();
    let key = parts.pop().filter(|key| !key.is_empty())?;
    let mut accelerator = Vec::with_capacity(parts.len() + 1);

    for modifier in parts {
        accelerator.push(match modifier {
            "ctrl" => "Control",
            "alt" => "Alt",
            "shift" => "Shift",
            "meta" => "Super",
            _ => return None,
        });
    }

    accelerator.push(key);
    accelerator.join("+").parse().ok()
}

/// Takes a page's bindings and registers them if its server is the one last shown. Returns what
/// could not be registered, for the page to show beside each binding.
pub fn set(app: &AppHandle, entry_id: &str, bindings: Vec<Binding>) -> Vec<BindingError> {
    let mut errors = Vec::new();
    let mut accepted = Vec::new();

    for binding in bindings.into_iter().take(MAX_BINDINGS) {
        if binding.action_id.len() > MAX_ACTION_ID || binding.combo.len() > MAX_COMBO {
            continue;
        }

        let Some(shortcut) = shortcut_for(&binding.combo) else {
            errors.push(BindingError {
                action_id: binding.action_id,
                reason: "unsupported",
            });

            continue;
        };

        if hotkey::is_shivers(&shortcut) {
            errors.push(BindingError {
                action_id: binding.action_id,
                reason: "taken",
            });

            continue;
        }

        accepted.push((binding.action_id, shortcut));
    }

    let state = app.state::<PageHotkeys>();

    state
        .0
        .locked()
        .bindings
        .insert(entry_id.to_string(), accepted);

    let failed = sync(app);
    let is_active = state.0.locked().active.as_deref() == Some(entry_id);

    if is_active {
        let bindings = state.0.locked().bindings.get(entry_id).cloned();

        for (action_id, shortcut) in bindings.unwrap_or_default() {
            if failed.contains(&shortcut) {
                errors.push(BindingError {
                    action_id,
                    reason: "taken",
                });
            }
        }
    }

    errors
}

/// Moves the registered bindings to the server now last shown, if that changed (called on a timer).
pub fn follow_active(app: &AppHandle) {
    let current = app.state::<webviews::ActiveServer>().get();
    let state = app.state::<PageHotkeys>();

    if state.0.locked().active == current {
        return;
    }

    state.0.locked().active = current;
    sync(app);
}

/// Forgets a removed server's bindings.
pub fn forget_entry(app: &AppHandle, entry_id: &str) {
    if app
        .state::<PageHotkeys>()
        .0
        .locked()
        .bindings
        .remove(entry_id)
        .is_some()
    {
        sync(app);
    }
}

/// Registers exactly the active server's bindings. Returns those another application holds.
///
/// The state is not held while registering, which goes through the event loop, where a press
/// waits on it; `SYNCING` keeps two syncs from interleaving instead.
fn sync(app: &AppHandle) -> Vec<Shortcut> {
    static SYNCING: Mutex<()> = Mutex::new(());

    let _serial = SYNCING.locked();
    let shortcuts = app.global_shortcut();
    let hotkeys = app.state::<PageHotkeys>();
    let (wanted, held) = {
        let mut state = hotkeys.0.locked();
        let wanted: Vec<Shortcut> = state
            .active
            .as_ref()
            .and_then(|active| state.bindings.get(active))
            .into_iter()
            .flatten()
            .map(|(_, shortcut)| *shortcut)
            .collect();

        (wanted, std::mem::take(&mut state.registered))
    };
    let mut registered = Vec::new();
    let mut failed = Vec::new();

    for shortcut in held {
        if wanted.contains(&shortcut) {
            registered.push(shortcut);
        } else {
            // a release no longer reaches an unregistered shortcut, so it is told now
            release(app, &shortcut);
            let _ = shortcuts.unregister(shortcut);
        }
    }

    for shortcut in wanted {
        if registered.contains(&shortcut) || failed.contains(&shortcut) {
            continue;
        }

        let handle = app.clone();
        let outcome = shortcuts.on_shortcut(shortcut, move |_app, _shortcut, event| {
            match event.state() {
                ShortcutState::Pressed => press(&handle, &shortcut),
                ShortcutState::Released => release(&handle, &shortcut),
            }
        });

        match outcome {
            Ok(()) => registered.push(shortcut),
            Err(_) => failed.push(shortcut),
        }
    }

    hotkeys.0.locked().registered = registered;

    failed
}

/// Tells the active server's page which of its actions a press was for.
fn press(app: &AppHandle, shortcut: &Shortcut) {
    let hotkeys = app.state::<PageHotkeys>();
    let (entry_id, actions) = {
        let mut state = hotkeys.0.locked();
        let Some(active) = state.active.clone() else {
            return;
        };
        let actions: Vec<String> = state
            .bindings
            .get(&active)
            .into_iter()
            .flatten()
            .filter(|(_, bound)| bound == shortcut)
            .map(|(action_id, _)| action_id.clone())
            .collect();

        state
            .held
            .insert(*shortcut, (active.clone(), actions.clone()));

        (active, actions)
    };

    tell(app, &entry_id, &actions, "down");
}

/// Tells the page that had a shortcut's press that it came back up.
fn release(app: &AppHandle, shortcut: &Shortcut) {
    let held = app.state::<PageHotkeys>().0.locked().held.remove(shortcut);

    if let Some((entry_id, actions)) = held {
        tell(app, &entry_id, &actions, "up");
    }
}

fn tell(app: &AppHandle, entry_id: &str, actions: &[String], phase: &str) {
    let Some(webview) = app.get_webview(&webviews::webview_label(entry_id)) else {
        return;
    };

    for action_id in actions {
        let action_id = serde_json::json!(action_id);
        let _ = webview.eval(format!(
            "window.__SHIVER_HOTKEY__ && window.__SHIVER_HOTKEY__({action_id}, '{phase}')"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharkords_combinations_read_as_shortcuts() {
        assert_eq!(
            shortcut_for("ctrl+shift+KeyM"),
            "Control+Shift+KeyM".parse().ok()
        );
        assert_eq!(shortcut_for("meta+F5"), "Super+F5".parse().ok());
        assert_eq!(shortcut_for("alt+ArrowUp"), "Alt+ArrowUp".parse().ok());
        assert_eq!(shortcut_for("Numpad7"), "Numpad7".parse().ok());
        assert!(shortcut_for("F24").is_some());

        assert_eq!(shortcut_for(""), None);
        assert_eq!(shortcut_for("ctrl+"), None);
        assert_eq!(
            shortcut_for("hyper+KeyM"),
            None,
            "a modifier Sharkord does not write"
        );
        assert_eq!(shortcut_for("ctrl+NotAKey"), None);
    }
}
