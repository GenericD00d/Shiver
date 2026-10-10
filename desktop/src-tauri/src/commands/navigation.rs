//! Showing servers, conversations and channels.

use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    drain::{self, Readiness},
    error::Result,
    session,
    store::{RegistryStore, Store},
    webviews,
};

use super::{page_inputs, voice_locked_for};

/// Shows a server, creating its page on first use, and remembers it for next launch.
///
/// Async: creating a webview blocks on the event loop, which a sync command (on the main thread)
/// would deadlock.
#[tauri::command]
pub async fn select_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<()> {
    let (entry, settings, muted) = page_inputs(&store, &id)?;
    let token = session::token_for_new_page(&app, &entry).await;

    webviews::show_server(
        &app,
        &entry,
        &settings,
        token.as_deref(),
        &muted,
        voice_locked_for(&app, &id),
    )?;
    webviews::trim_pages(&app);
    crate::watch::publish_floor(&app, &id);

    store.update(|registry| {
        registry.settings.last_server_id = Some(id);

        Ok(())
    })
}

/// Brings a server's page up hidden and says whether its client is already connected, so the shell
/// can cover it until `shiver://server-ready`.
#[tauri::command]
pub async fn prepare_server(app: AppHandle, store: State<'_, Store>, id: String) -> Result<bool> {
    if app.get_webview(&webviews::webview_label(&id)).is_some() {
        // a page that will not recover by itself is reloaded rather than waited on ("Try again")
        crate::drain::revive_now(&app, &id);
    } else {
        let (entry, settings, muted) = page_inputs(&store, &id)?;
        let token = session::token(&app, &entry).await;

        webviews::preload_server(
            &app,
            &entry,
            &settings,
            token.as_deref(),
            &muted,
            voice_locked_for(&app, &id),
        )?;

        // the page reports for itself now, so its socket stands down
        crate::watch::sync(&app);
    }

    Ok(app.state::<Readiness>().is_ready(&id))
}

#[tauri::command]
pub async fn show_shell(app: AppHandle) -> Result<()> {
    webviews::show_shell_only(&app)
}

#[tauri::command]
pub async fn exit_dm_split(app: AppHandle) {
    webviews::end_conversation(&app);
}

/// Opens one conversation in the entry's own page, beside Shiver's DM list.
#[tauri::command]
pub async fn open_dm(
    app: AppHandle,
    store: State<'_, Store>,
    entry_id: String,
    name: String,
) -> Result<()> {
    let (entry, settings, muted) = page_inputs(&store, &entry_id)?;
    let token = session::token_for_new_page(&app, &entry).await;

    webviews::show_conversation(
        &app,
        &entry,
        &settings,
        token.as_deref(),
        &muted,
        voice_locked_for(&app, &entry_id),
        &name,
    )?;

    // also stands the entry's socket down, should its page have just been opened
    webviews::trim_pages(&app);

    Ok(())
}

/// From the popup: asks the shell to go to a message, and closes the popup.
#[tauri::command]
pub async fn open_message(
    app: AppHandle,
    entry_id: String,
    channel_id: Option<i64>,
    is_dm: bool,
    author: String,
) -> Result<()> {
    let _ = app.emit_to(
        webviews::SHELL_WEBVIEW,
        drain::OPEN_MESSAGE_EVENT,
        serde_json::json!({ "entryId": entry_id, "channelId": channel_id, "isDm": is_dm, "author": author }),
    );

    webviews::set_popup_open(&app, false)
}

/// Puts a server's page on one channel (the shell has already opened the server).
#[tauri::command]
pub fn select_channel(app: AppHandle, entry_id: String, channel_id: i64) -> Result<()> {
    if let Some(webview) = app.get_webview(&webviews::webview_label(&entry_id)) {
        webview.eval(format!(
            "window.__SHIVER_SELECT_CHANNEL__ && window.__SHIVER_SELECT_CHANNEL__({channel_id})"
        ))?;
    }

    Ok(())
}
