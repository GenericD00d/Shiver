//! Telling the server pages things: what Shiver evaluates in them (theme, visibility, mutes, voice
//! locks and actions, notification levels), only ever into the entry's own page.

use serde_json::json;
use shiver_core::model::NotifyLevel;
use tauri::{AppHandle, Manager};

use super::{entry_of, main_window, webview_label};
use crate::model::Settings;

/// Evaluates `script` in every server page.
fn eval_in_pages(app: &AppHandle, script: &str) {
    let Ok(window) = main_window(app) else {
        return;
    };

    for webview in window.webviews() {
        if entry_of(webview.label()).is_some() {
            let _ = webview.eval(script);
        }
    }
}

/// Tells pages whether the window is minimised; WebView2 does not change `document.hidden` for it,
/// and Sharkord only notifies for the selected channel when the page is hidden.
pub fn push_visibility(app: &AppHandle, hidden: bool) {
    eval_in_pages(
        app,
        &format!("window.__SHIVER_SET_HIDDEN__ && window.__SHIVER_SET_HIDDEN__({hidden})"),
    );
}

/// Repaints every page in the user's colours and applies the sound and attachment settings, in place.
pub fn push_theme(app: &AppHandle, settings: &Settings) {
    let theme = theme_payload(settings);
    let volume = settings.sound_volume.min(crate::model::MAX_SOUND_VOLUME);
    let minimise = settings.minimise_attachments;

    eval_in_pages(
        app,
        &format!(
            "window.__SHIVER_SET_THEME__ && window.__SHIVER_SET_THEME__({theme});\
             window.__SHIVER_SET_SOUND_VOLUME__ && window.__SHIVER_SET_SOUND_VOLUME__({volume});\
             window.__SHIVER_SET_ATTACHMENT_CARDS__ && window.__SHIVER_SET_ATTACHMENT_CARDS__({minimise});"
        ),
    );
}

/// The colour a Shiver webview paints before its stylesheet loads, from the user's background.
pub(super) fn startup_color(settings: &Settings) -> tauri::webview::Color {
    let [r, g, b] = shiver_core::model::rgb(&settings.theme_color).unwrap_or([0x17; 3]);

    tauri::webview::Color(r, g, b, 255)
}

pub(super) fn theme_payload(settings: &Settings) -> serde_json::Value {
    shiver_core::model::theme_payload(
        &settings.theme_color,
        &settings.accent_color,
        settings.text_color.as_deref(),
    )
}

fn eval_in(app: &AppHandle, label: &str, script: &str) {
    if let Some(webview) = app.get_webview(label) {
        let _ = webview.eval(script);
    }
}

/// Tells one page which of its notifications Shiver wants.
pub fn push_notify_level(app: &AppHandle, entry_id: &str, level: NotifyLevel) {
    eval_in(
        app,
        &webview_label(entry_id),
        &format!(
            "window.__SHIVER_SET_NOTIFY__ && window.__SHIVER_SET_NOTIFY__({})",
            json!(level)
        ),
    );
}

/// Tells one page whether a call is running elsewhere (never where).
pub fn push_voice_lock(app: &AppHandle, entry_id: &str, locked: bool) {
    eval_in(
        app,
        &webview_label(entry_id),
        &format!("window.__SHIVER_SET_VOICE_LOCK__ && window.__SHIVER_SET_VOICE_LOCK__({locked})"),
    );
}

/// Clicks one of Sharkord's own voice controls in a page.
pub fn run_voice_action(app: &AppHandle, entry_id: &str, action: &str) {
    eval_in(
        app,
        &webview_label(entry_id),
        &format!(
            "window.__SHIVER_VOICE__ && window.__SHIVER_VOICE__({})",
            json!(action)
        ),
    );
}

pub fn mark_all_read(app: &AppHandle, entry_id: &str) {
    eval_in(
        app,
        &webview_label(entry_id),
        "window.__SHIVER_MARK_ALL_READ__ && window.__SHIVER_MARK_ALL_READ__()",
    );
}

/// Tells one page its mute list changed.
pub fn push_muted(app: &AppHandle, entry_id: &str, muted: &[i64]) {
    eval_in(
        app,
        &webview_label(entry_id),
        &format!(
            "window.__SHIVER_SET_MUTED__ && window.__SHIVER_SET_MUTED__({})",
            json!(muted)
        ),
    );
}
