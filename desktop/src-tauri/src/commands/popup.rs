//! The bell's popup.

use tauri::{AppHandle, Manager};

use crate::{error::Result, webviews};

/// Toggles the popup; returns the new state.
#[tauri::command]
pub async fn toggle_popup(app: AppHandle) -> Result<bool> {
    let open = app.state::<webviews::ActiveServer>().should_open_popup();

    webviews::set_popup_open(&app, open)?;

    Ok(open)
}

#[tauri::command]
pub async fn close_popup(app: AppHandle) -> Result<()> {
    webviews::set_popup_open(&app, false)
}

/// The popup lost focus; arms the reopen guard so the click that caused it does not reopen it.
#[tauri::command]
pub async fn dismiss_popup(app: AppHandle) -> Result<()> {
    app.state::<webviews::ActiveServer>().note_popup_dismissed();
    webviews::set_popup_open(&app, false)
}
