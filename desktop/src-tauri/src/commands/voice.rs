//! The call bar: who holds the call, and its controls.

use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    drain,
    error::{Core, Result},
    voice::{VoiceState, VoiceStatus},
    webviews,
};

#[tauri::command]
pub fn voice_status(voice: State<'_, VoiceState>) -> Option<VoiceStatus> {
    voice.status()
}

/// Works the call's controls on whichever entry holds it.
#[tauri::command]
pub async fn voice_control(app: AppHandle, action: String) -> Result<()> {
    if !matches!(action.as_str(), "mic" | "sound" | "leave") {
        return Err(Core::InvalidInput(format!("Unknown voice action '{action}'")).into());
    }

    let voice = app.state::<VoiceState>();

    if let Some(holder) = voice.holder() {
        webviews::run_voice_action(&app, &holder, &action);

        // a page that does not really leave loses the call anyway; only the page on screen can
        // start one again
        if action == "leave" {
            voice.forget_entry(&holder);
            let _ = app.emit_to(webviews::SHELL_WEBVIEW, drain::VOICE_EVENT, ());
        }
    }

    Ok(())
}
