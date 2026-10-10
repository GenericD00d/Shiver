//! Settings, the version, and the trusted links.

use tauri::{AppHandle, Emitter, State};

use crate::{
    error::{Core, Result},
    hotkey,
    model::Settings,
    store::{RegistryStore, Store},
    webviews,
};

/// Tells Shiver's own webviews the settings changed.
const SETTINGS_EVENT: &str = "shiver://settings";

/// Makes every site whose links opened without asking ask again.
#[tauri::command]
pub async fn forget_trusted_links(store: State<'_, Store>) -> Result<()> {
    store.update(|registry| {
        registry.settings.trusted_links.clear();

        Ok(())
    })
}

#[tauri::command]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
pub fn get_settings(store: State<'_, Store>) -> Settings {
    store.registry().settings.clone().sanitised()
}

/// Saves settings (sanitised; Shiver's own bookkeeping fields are kept) and applies them live.
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    store: State<'_, Store>,
    settings: Settings,
) -> Result<()> {
    if settings
        .mute_hotkey
        .as_deref()
        .is_some_and(hotkey::is_rail_shortcut)
    {
        return Err(Core::InvalidInput(
            "That shortcut opens a server from the rail (Ctrl+1 to 9, Ctrl+Alt+Up or Down). Choose another for muting.".into(),
        )
        .into());
    }

    let before = store.registry().settings.start_at_login;
    let saved = store.update(|registry| {
        registry.settings = Settings {
            last_server_id: registry.settings.last_server_id.take(),
            skipped_update: registry.settings.skipped_update.take(),
            trusted_links: std::mem::take(&mut registry.settings.trusted_links),
            ..settings.sanitised()
        };

        Ok(registry.settings.clone())
    })?;

    if saved.start_at_login != before {
        crate::tray::set_start_at_login(&app, saved.start_at_login);
    }

    crate::tray::apply(&app, saved.close_to_tray);
    hotkey::apply(&app, saved.mute_hotkey.as_deref());
    webviews::trim_pages(&app);
    webviews::push_theme(&app, &saved);

    for label in [
        webviews::SHELL_WEBVIEW,
        webviews::OVERLAY_WEBVIEW,
        webviews::POPUP_WEBVIEW,
    ] {
        let _ = app.emit_to(label, SETTINGS_EVENT, ());
    }

    Ok(())
}
