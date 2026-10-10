//! Settings, the version, and the trusted links.

use tauri::{AppHandle, State};

use crate::{
    error::Result,
    model::Settings,
    store::{RegistryStore, Store},
    webview,
};

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

/// Saves settings (sanitised; bookkeeping fields kept) and repaints the gap between pages.
#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    store: State<'_, Store>,
    settings: Settings,
) -> Result<()> {
    let saved = store.update(|registry| {
        registry.settings = Settings {
            last_server_id: registry.settings.last_server_id.take(),
            skipped_update: registry.settings.skipped_update.take(),
            push_servers: std::mem::take(&mut registry.settings.push_servers),
            trusted_links: std::mem::take(&mut registry.settings.trusted_links),
            ..settings.sanitised()
        };

        Ok(registry.settings.clone())
    })?;

    if let Ok(window) = webview::main_window(&app) {
        let _ = window.set_background_color(Some(webview::background_color(&saved)));
    }

    Ok(())
}
