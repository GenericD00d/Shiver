mod badge;
mod badges;
mod commands;
mod drain;
mod error;
mod feed;
mod hotkey;
mod jwt;
mod model;
mod permissions;
mod secrets;
mod session;
mod store;
mod update;
mod voice;
mod watch;
mod webviews;

use tauri::{Emitter, Manager};

use crate::{
    drain::Readiness, feed::Feed, session::Recovery, store::Store, voice::VoiceState,
    watch::Watcher, webviews::ActiveServer,
};

/// Shows why Shiver could not start (a Windows GUI app has no console for stderr), then exits.
fn report_failed_start(error: tauri::Error) -> ! {
    let message = format!("Shiver could not start: {error}");

    eprintln!("[shiver] {message}");

    #[cfg(windows)]
    unsafe {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(
                window: *mut core::ffi::c_void,
                text: *const u16,
                caption: *const u16,
                kind: u32,
            ) -> i32;
        }

        const MB_ICONERROR: u32 = 0x10;

        let wide = |text: &str| {
            text.encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let (text, caption) = (wide(&message), wide("Shiver"));

        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_ICONERROR,
        );
    }

    std::process::exit(1);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::list_registry,
            commands::reset_media_permissions,
            commands::forget_password,
            update::install_update,
            update::available_update,
            update::skip_update,
            update::open_repository,
            update::check_for_update,
            commands::check_server,
            commands::add_server,
            commands::remove_server,
            commands::reorder_servers,
            commands::set_server_folder,
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
            commands::set_accept_any_size,
            commands::mark_server_read,
            commands::mark_notifications_read,
            commands::clear_notifications,
            commands::set_channel_muted,
            commands::show_server_menu,
            commands::toggle_popup,
            commands::close_popup,
            commands::dismiss_popup,
            commands::open_dm,
            commands::select_channel,
            commands::open_message,
            commands::exit_dm_split,
            commands::reorder_rail,
            commands::create_folder_with,
            commands::show_folder_menu,
        ])
        .setup(|app| {
            let handle = app.handle();

            app.manage(store::load(handle)?);
            app.manage(ActiveServer::default());
            app.manage(Feed::default());
            app.manage(VoiceState::default());
            app.manage(Readiness::default());
            app.manage(Recovery::default());
            app.manage(drain::Broadcast::default());
            app.manage(webviews::Openings::default());
            app.manage(update::Announced::default());
            app.manage(update::Available::default());
            app.manage(Watcher::default());
            app.manage(watch::Missed::default());
            app.manage(watch::Plugins::default());
            app.manage(watch::Reported::default());
            app.manage(watch::ReadStates::default());

            let mute_hotkey = app.state::<Store>().registry().settings.mute_hotkey.clone();

            hotkey::apply(handle, mute_hotkey.as_deref());

            webviews::create_main_window(handle)?;
            webviews::prune_profiles(handle);
            drain::spawn(handle);
            crate::watch::sync(handle);

            update::start(handle);

            Ok(())
        })
        .on_menu_event(|app, event| {
            // native menu item ids are `action:entry`; the shell performs the action
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
        .unwrap_or_else(|error| report_failed_start(error));
}
