mod badge;
mod commands;
mod drain;
mod error;
mod feed;
mod hotkey;
mod jwt;
mod login;
mod model;
mod permissions;
mod probe;
mod secrets;
mod session;
mod store;
mod voice;
mod webviews;

use tauri::{Emitter, Manager};

use crate::{
    drain::Readiness, feed::Feed, session::Recovery, store::Store, voice::VoiceState,
    webviews::ActiveServer,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::list_registry,
            commands::reset_media_permissions,
            commands::probe_server,
            commands::add_server,
            commands::remove_server,
            commands::reorder_servers,
            commands::set_server_folder,
            commands::create_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::set_folder_expanded,
            commands::select_server,
            commands::prepare_server,
            commands::voice_status,
            commands::voice_control,
            commands::show_shell,
            commands::app_version,
            commands::get_settings,
            commands::update_settings,
            commands::refresh_server_info,
            commands::sign_in_server,
            commands::log_out_server,
            commands::list_notifications,
            commands::list_dms,
            commands::unread_count,
            commands::unread_counts,
            commands::mark_server_read,
            commands::mark_notifications_read,
            commands::clear_notifications,
            commands::set_channel_muted,
            commands::show_server_menu,
            commands::toggle_popup,
            commands::close_popup,
            commands::dismiss_popup,
            commands::open_dm,
            commands::exit_dm_split,
            commands::reorder_rail,
            commands::create_folder_with,
            commands::show_folder_menu,
        ])
        .setup(|app| {
            let handle = app.handle();

            app.manage(Store::load(handle)?);
            app.manage(ActiveServer::default());
            app.manage(Feed::default());
            app.manage(VoiceState::default());
            app.manage(Readiness::default());
            app.manage(Recovery::default());
            app.manage(drain::Broadcast::default());
            app.manage(drain::Openings::default());

            // registered from the settings Shiver just loaded, so the shortcut works from launch
            // rather than from the first time the settings screen is opened
            let mute_hotkey = app.state::<Store>().registry().settings.mute_hotkey.clone();

            hotkey::apply(handle, mute_hotkey.as_deref());

            webviews::create_main_window(handle)?;
            drain::spawn(handle);

            // off the main thread: creating a webview blocks on the event loop, and setup *is* the
            // event loop. it also lets the window paint before the servers start connecting.
            let preload = handle.clone();

            tauri::async_runtime::spawn(commands::preload_all_servers(preload));

            Ok(())
        })
        .on_menu_event(|app, event| {
            // the rail's native context menu encodes its target in the item id, so the shell is
            // told what was chosen and performs it through the same commands the ui already uses
            let id = event.id().0.as_str();

            let Some((action, entry_id)) = id.split_once(':') else {
                return;
            };

            let _ = app.emit_to(
                webviews::SHELL_WEBVIEW,
                "shiver://server-menu",
                serde_json::json!({ "action": action, "entryId": entry_id }),
            );
        })
        .run(tauri::generate_context!())
        .expect("Shiver failed to start");
}
