mod commands;
mod error;
mod icons;
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

/// Android attributes a call to the page last reported loaded, which lags a new page, so commands
/// are refused whenever anything but Shiver's own page is on screen or being opened.
fn only_home(
    handler: impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    move |invoke| {
        if invoke.message.webview_ref().state::<Showing>().at_home() {
            return handler(invoke);
        }

        invoke
            .resolver
            .reject("Only Shiver's own page can call Shiver");

        true
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shiver_secrets::init())
        .plugin(tauri_plugin_shiver_push::init())
        .invoke_handler(only_home(tauri::generate_handler![
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
            commands::set_accept_any_size,
            commands::app_version,
            commands::update_settings,
            commands::forget_trusted_links,
            commands::reorder_rail,
            commands::create_folder_with,
            commands::set_server_folder,
            commands::rename_folder,
            commands::delete_folder,
            commands::set_folder_expanded,
            commands::unread_counts,
            commands::session_states,
            commands::list_dms,
            commands::refresh_server_info,
            commands::server_icons,
            commands::server_still,
            commands::log_out_server,
            commands::forget_sessions,
            commands::push_status,
            commands::set_push_server,
            commands::set_push_distributor,
        ]))
        .setup(|app| {
            let handle = app.handle().clone();

            app.manage(store::load(&handle)?);
            app.manage(webview::Openings::default());
            app.manage(Showing::default());
            app.manage(sharkord::CheckedSessions::default());
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
            .initialization_script(webview::document_start())
            .on_navigation({
                let handle = handle.clone();

                // Allowed is not arrived: Android asks here for frames inside a page too, and for
                // navigations the page then cancels, so arrival is noticed on load (below).
                move |target| {
                    let server_origin = current_origin(&handle);

                    if webview::is_allowed(&handle, target, server_origin.as_deref()) {
                        return true;
                    }

                    let away = target.origin().ascii_serialization();

                    eprintln!("[shiver] the page tried to leave for {away}; refused");

                    if handle.state::<Showing>().loading() {
                        webview::show_failed(&handle);
                    }

                    false
                }
            })
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_page_load({
                let handle = handle.clone();

                move |_window, payload| {
                    let url = payload.url();

                    if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                        return install_bridge_if_server(&handle, url);
                    }

                    // A load that started in the main frame, however it was asked for (a page, the
                    // core's `navigate`, a step through history): where home is noticed, and where
                    // a page that is neither Shiver's nor the server it opened is sent home. The
                    // very first load is Shiver's own page.
                    handle.state::<Showing>().set_home_if_unset(url.to_string());

                    if webview::is_home(&handle, url) {
                        webview::landed_home(&handle);
                    } else if matches!(url.scheme(), "http" | "https")
                        && !current_origin(&handle)
                            .is_some_and(|origin| model::is_same_origin(&origin, url))
                    {
                        eprintln!(
                            "[shiver] {} loaded without Shiver opening it; going home",
                            url.origin().ascii_serialization()
                        );
                        handle.state::<Showing>().set_stray();
                        webview::go_home(&handle, None);
                    }
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
            icons::restore(&handle);

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

    let (entry, settings, muted) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let Some(entry) = registry.server(&entry_id).cloned() else {
            return;
        };

        (
            entry,
            registry.settings.clone(),
            registry.muted_for(&entry_id),
        )
    };

    // Shiver's own pages load in this webview too
    if !model::is_same_origin(&entry.origin, url) {
        return;
    }

    app.state::<Showing>().set_loaded();

    let inbox = app.state::<Inbox>();
    let session = inbox.token(&entry_id);
    let read_floor = inbox.baseline(&entry_id);
    let push_endpoint = app.state::<push::Push>().endpoint(&entry_id);
    let retired: Vec<String> = entry
        .retired_push_endpoints
        .iter()
        .filter(|endpoint| push_endpoint.as_ref() != Some(*endpoint))
        .cloned()
        .collect();

    if !entry.retired_push_endpoints.is_empty() {
        let (handle, id) = (app.clone(), entry_id.clone());

        tauri::async_runtime::spawn_blocking(move || {
            handle.state::<Store>().update(|registry| {
                if let Some(server) = registry.server_mut(&id) {
                    server.retired_push_endpoints.clear();
                }

                Ok(())
            })
        });
    }

    webview::install_bridge(
        app,
        webview::PageContext {
            entry: &entry,
            settings: &settings,
            muted: &muted,
            session: session.as_deref(),
            read_floor: read_floor.as_ref(),
            push_endpoint: push_endpoint.as_deref(),
            retired_push_endpoints: &retired,
        },
    );

    // the signed-in page's session lets Shiver keep watching this server after the user leaves
    inbox::harvest_token(app, entry_id);
}

/// Opens a link the server's page asked for, once the user agrees in a native dialog (the page
/// cannot draw over it), or at once for a site they chose to trust. One question at a time; links
/// asked for meanwhile are dropped. Android may hand an https link to another app.
pub fn ask_to_open(app: &AppHandle, server: &str, url: &Url) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

    static ASKING: AtomicBool = AtomicBool::new(false);

    let url = webview::without_seed(url);

    let Some(site) = shiver_core::links::site(&url) else {
        return;
    };

    if app
        .state::<Store>()
        .registry()
        .settings
        .trusted_link_sites
        .contains(&site)
    {
        return open_externally(app, &url);
    }

    if ASKING.swap(true, Ordering::AcqRel) {
        return eprintln!("[shiver] {server} asked to open a link while another waited; dropped");
    }

    let always = format!("Always for {site}");
    let app = app.clone();

    app.dialog()
        .message(shiver_core::links::question(server, &site, &url))
        .title("Open link?")
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            "Open".into(),
            always.clone(),
            "Cancel".into(),
        ))
        .show_with_result(move |answer| {
            ASKING.store(false, Ordering::Release);

            let MessageDialogResult::Custom(choice) = answer else {
                return;
            };

            if choice == always {
                let _ = app.state::<Store>().update(|registry| {
                    shiver_core::links::trust(&mut registry.settings.trusted_link_sites, site);

                    Ok(())
                });
            } else if choice != "Open" {
                return;
            }

            open_externally(&app, &url);
        });
}

fn open_externally(app: &AppHandle, url: &Url) {
    use tauri_plugin_opener::OpenerExt;

    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("[shiver] could not open {url} in the browser: {error}");
    }
}
