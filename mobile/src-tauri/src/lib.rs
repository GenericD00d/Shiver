mod commands;
mod error;
mod inbox;
mod model;
mod push;
pub use sharkord_client as sharkord;
mod store;
mod update;
mod webview;

use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

use crate::{
    inbox::Inbox,
    store::{RegistryStore, Store},
    webview::{Showing, MAIN_WINDOW},
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shiver_secrets::init())
        .plugin(tauri_plugin_shiver_push::init())
        .invoke_handler(tauri::generate_handler![
            update::update_available,
            update::open_releases,
            commands::forget_password,
            commands::list_registry,
            commands::probe_server,
            commands::check_server,
            update::skip_update,
            update::open_repository,
            update::check_for_update,
            commands::add_server,
            commands::remove_server,
            commands::select_server,
            commands::sign_in_server,
            commands::signed_out_servers,
            commands::watch_problems,
            commands::set_accept_any_size,
            commands::remembered_servers,
            commands::app_version,
            commands::update_settings,
            commands::reorder_rail,
            commands::create_folder_with,
            commands::set_server_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::set_folder_expanded,
            commands::unread_counts,
            commands::list_dms,
            commands::server_plugins,
            commands::refresh_server_info,
            commands::log_out_server,
            commands::forget_sessions,
            commands::push_status,
            commands::set_push_server,
            commands::set_push_distributor,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            app.manage(store::load(&handle)?);
            app.manage(webview::Openings::default());
            app.manage(Showing::default());
            app.manage(Inbox::default());
            app.manage(push::Push::default());
            app.manage(update::Available::default());

            let settings = handle.state::<Store>().registry().settings.clone();

            // built here rather than in the config, so the navigation guard and page-load hook
            // can be attached
            let window = WebviewWindowBuilder::new(
                &handle,
                MAIN_WINDOW,
                WebviewUrl::App("index.html".into()),
            )
            .title("Shiver")
            .inner_size(420.0, 860.0)
            .background_color(webview::background_color(&settings))
            .initialization_script(webview::DOCUMENT_START)
            .on_navigation({
                let handle = handle.clone();

                move |target| {
                    let server_origin = current_origin(&handle);

                    if webview::is_allowed(&handle, target, server_origin.as_deref()) {
                        return true;
                    }

                    open_externally(&handle, target);

                    false
                }
            })
            .on_new_window({
                let handle = handle.clone();

                // `target="_blank"` links; Android does not ask, so its bridge queues them instead
                move |url, _features| {
                    open_externally(&handle, &url);

                    tauri::webview::NewWindowResponse::Deny
                }
            })
            .on_page_load({
                let handle = handle.clone();

                move |_window, payload| {
                    if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                        return;
                    }

                    if webview::is_home(&handle, payload.url()) {
                        webview::landed_home(&handle);
                    }

                    install_bridge_if_server(&handle, payload.url());
                }
            })
            .build()?;

            // fallback for a platform whose opening navigation never reaches the guard
            if let Ok(url) = window.url() {
                app.state::<Showing>().set_home_if_unset(url.to_string());
            }

            let _ = app
                .state::<Store>()
                .update(|registry| Ok(registry.rail().prune_folders()));

            inbox::restore(&handle);
            ask_to_notify(&handle);

            // tokens before listening, listening before registering: the distributor answers a
            // registration with a broadcast
            push::migrate_tokens(&handle);
            push::start(&handle);
            push::register_wanted(&handle);

            inbox::watch_mutes(&handle);
            update::start(&handle);
            backfill_icons(handle);

            Ok(())
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            eprintln!("[shiver] Shiver could not start: {error}");
            std::process::exit(1);
        });
}

/// Asks for notification permission once, off the main thread (a JVM call from the setup hook
/// would deadlock before the first frame).
fn ask_to_notify(app: &AppHandle) {
    use tauri_plugin_notification::{NotificationExt, PermissionState};

    let app = app.clone();

    std::thread::spawn(move || {
        let notification = app.notification();

        match notification.permission_state() {
            Ok(PermissionState::Granted) => {}
            Ok(_) => {
                if let Err(error) = notification.request_permission() {
                    eprintln!("[shiver] could not ask about notifications: {error}");
                }
            }
            Err(error) => eprintln!("[shiver] could not read the notification permission: {error}"),
        }
    });
}

/// The origin of the server on screen, if any.
fn current_origin(app: &AppHandle) -> Option<String> {
    let entry_id = app.state::<Showing>().server()?;

    app.state::<Store>()
        .registry()
        .server(&entry_id)
        .map(|server| server.origin.clone())
}

/// Installs the bridge once a page on the open server's origin has finished loading.
fn install_bridge_if_server(app: &AppHandle, url: &Url) {
    let Some(entry_id) = app.state::<Showing>().server() else {
        return;
    };

    let (entry, settings, muted, servers, folders) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let Some(entry) = registry.server(&entry_id).cloned() else {
            return;
        };

        (
            entry,
            registry.settings.clone(),
            registry.muted_for(&entry_id),
            registry.servers.clone(),
            registry.folders.clone(),
        )
    };

    // Shiver's own pages load in this webview too
    if !model::is_same_origin(&entry.origin, url) {
        return;
    }

    let inbox = app.state::<Inbox>();
    let unread = inbox.unread();
    let signed_out = inbox.signed_out();
    let session = inbox.token(&entry_id);
    let read_floor = inbox.baseline(&entry_id);
    let carried = inbox.carried(&entry_id);
    let push_endpoint = app.state::<push::Push>().endpoint(&entry_id);

    webview::install_bridge(
        app,
        webview::PageContext {
            entry: &entry,
            settings: &settings,
            muted: &muted,
            servers: &servers,
            unread: &unread,
            signed_out: &signed_out,
            session: session.as_deref(),
            carried: carried.as_deref(),
            read_floor: read_floor.as_ref(),
            folders: &folders,
            push_endpoint: push_endpoint.as_deref(),
        },
    );

    // the signed-in page's session lets Shiver keep watching this server after the user leaves
    inbox::harvest_token(app, entry_id);
}

/// Inlines the logos of servers added before Shiver stored them, once at startup.
fn backfill_icons(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let missing: Vec<(String, String)> = app
            .state::<Store>()
            .registry()
            .servers
            .iter()
            .filter(|server| server.icon_data.is_none())
            .filter_map(|server| Some((server.id.clone(), server.icon_url.clone()?)))
            .collect();

        for (id, url) in missing {
            let Some(data) = shiver_core::probe::fetch_icon(&url).await else {
                continue;
            };

            let _ = app.state::<Store>().update(|registry| {
                if let Some(server) = registry.server_mut(&id) {
                    server.icon_data = Some(data);
                }

                Ok(())
            });
        }
    });
}

/// Hands an http(s) link to the system browser.
pub fn open_externally(app: &AppHandle, url: &Url) {
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }

    use tauri_plugin_opener::OpenerExt;

    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("[shiver] could not open {url} in the browser: {error}");
    }
}
