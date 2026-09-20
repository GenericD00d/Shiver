mod commands;
mod error;
mod inbox;
mod login;
mod model;
mod probe;
mod push;
// The protocol lives in `shared/sharkord-client`, because desktop speaks it now too. Re-exported under
// the old path so every `crate::sharkord::` reference still reads the same.
pub use sharkord_client as sharkord;
mod store;
mod update;
mod webview;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

use crate::{
    inbox::Inbox,
    store::{RegistryStore, Store},
    webview::{Showing, MAIN_WINDOW},
};

/// Says why Shiver could not start, rather than vanishing.
///
/// The release profile is `panic = "abort"` with `strip = true`, and on Windows there is no console
/// attached — so a failure to create the config directory or the window used to be an app that
/// disappeared with the reason written nowhere at all. Everything else in this codebase reports its
/// failures carefully; the one at startup did not.
fn report_failed_start(error: tauri::Error) -> ! {
    let message = format!("Shiver could not start: {error}");

    eprintln!("[shiver] {message}");

    // Windows is the case that actually needs this: a bundled app has no console attached, so
    // stderr goes nowhere a user will ever look. `MessageBoxW` is in `user32`, which every windows
    // process already has loaded — declared here rather than pulling in a dialog plugin for one
    // call on a path that by definition has no app to hang a plugin off.
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

        let text = wide(&message);
        let caption = wide("Shiver");

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
            commands::show_shiver,
            commands::showing_server,
            commands::sign_in_server,
            commands::signed_out_servers,
            commands::watch_problems,
            commands::set_accept_any_size,
            commands::remembered_servers,
            commands::app_version,
            commands::get_settings,
            commands::update_settings,
            commands::reorder_servers,
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
            commands::mark_server_read,
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

            // built here rather than declared in the config so the navigation guard and the page
            // load hook can be attached to it, which the config cannot express
            // read before the window is built, because the background colour below is the user's
            let settings = handle.state::<Store>().registry().settings.clone();

            let window = WebviewWindowBuilder::new(
                &handle,
                MAIN_WINDOW,
                WebviewUrl::App("index.html".into()),
            )
            .title("Shiver")
            .inner_size(420.0, 860.0)
            // What shows while one page is being replaced by the next, which on mobile is every
            // server switch — and it is white by default, on a client that is black everywhere.
            // See `webview::background_color`.
            .background_color(webview::background_color(&settings))
            // And what the *next* page paints before its own theme is decided, which is white for
            // the same reason on every Sharkord page. A document-start script, which Android does
            // support — this is not the bridge and does not move it. See
            // `webview::THEME_BEFORE_FIRST_PAINT`.
            .initialization_script(webview::THEME_BEFORE_FIRST_PAINT)
            .on_navigation({
                let handle = handle.clone();

                move |target| {
                    let server_origin = current_origin(&handle);

                    if webview::is_allowed(&handle, target, server_origin.as_deref()) {
                        return true;
                    }

                    // a link out of Sharkord: the browser's job, not this webview's, so a server
                    // cannot move the one webview Shiver has somewhere it still labels as that server
                    open_externally(&handle, target);

                    false
                }
            })
            .on_new_window({
                let handle = handle.clone();

                // Sharkord opens files and links with `target="_blank"`, which is a new window
                // rather than a navigation, so the guard above never saw them. Android's webview
                // drops such a request on the floor unless the host says otherwise, which is why
                // tapping an attachment or a link in a message did nothing at all.
                //
                // Answered here for the platforms that ask, and by the bridge for Android, which
                // does not — see `installExternalLinks`.
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

                    // Before the bridge, because it decides whether there is a server in play at
                    // all: arriving at Shiver's own pages is what ends the last one being "shown".
                    if webview::is_home(&handle, payload.url()) {
                        webview::landed_home(&handle);
                    }

                    install_bridge_if_server(&handle, payload.url());
                }
            })
            .build()?;

            // a fallback only: the opening navigation has almost certainly recorded home already,
            // and in development it is the more accurate of the two — this covers a platform where
            // that navigation never reaches the guard
            if let Ok(url) = window.url() {
                app.state::<Showing>().set_home_if_unset(url.to_string());
            }

            // sessions kept from previous runs, so the inbox has something to connect with before
            // the user has opened anything
            // Folders left over from before they knew how to dissolve, or from a server removed
            // while Shiver was closed. Cheap, and it means the rule holds from the first frame.
            let _ = app
                .state::<Store>()
                .update(|registry| Ok(model::prune_folders(registry)));

            inbox::restore(&handle);
            ask_to_notify(&handle);

            // Listening before registering: the distributor answers a registration with a broadcast,
            // so the handler has to be in place first or the first endpoint of the run is missed.
            push::start(&handle);
            push::register_wanted(&handle);

            // the muted channels live in whichever server's page is on screen, and the core needs
            // them before the user leaves it rather than after
            inbox::watch_mutes(&handle);

            update::start(&handle);

            backfill_icons(handle);

            Ok(())
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| report_failed_start(error));
}

/// The origin of the server the webview is on, if it is on one.
/// Asks Android for permission to post notifications, once, at startup.
///
/// Off the main thread, like every other call into the JVM: `run_mobile_plugin` blocks the caller
/// while Android answers on the main thread, so asking from the setup hook would deadlock the app
/// before it drew a frame.
///
/// A refusal is not an error worth reporting. Shiver keeps its badges either way, and Android will
/// not ask again — the user can turn notifications on in system settings whenever they want them.
fn ask_to_notify(app: &tauri::AppHandle) {
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

fn current_origin(app: &tauri::AppHandle) -> Option<String> {
    let entry_id = app.state::<Showing>().server()?;
    let store = app.state::<Store>();
    let registry = store.registry();

    registry
        .servers
        .iter()
        .find(|server| server.id == entry_id)
        .map(|server| server.origin.clone())
}

/// Runs the bridge once a page on the open server's own origin has finished loading.
fn install_bridge_if_server(app: &tauri::AppHandle, url: &tauri::Url) {
    let Some(entry_id) = app.state::<Showing>().server() else {
        return;
    };

    let (entry, settings, muted, servers, folders) = {
        let store = app.state::<Store>();
        let registry = store.registry();

        let Some(entry) = registry
            .servers
            .iter()
            .find(|server| server.id == entry_id)
            .cloned()
        else {
            return;
        };

        let muted = registry
            .muted
            .iter()
            .filter(|muted| muted.entry_id == entry_id)
            .map(|muted| muted.channel_id)
            .collect::<Vec<_>>();

        (
            entry,
            registry.settings.clone(),
            muted,
            registry.servers.clone(),
            registry.folders.clone(),
        )
    };

    // Shiver's own pages load in this webview too, and must not be handed a server's bridge
    if !model::is_same_origin(&entry.origin, url) {
        return;
    }

    let unread = app.state::<Inbox>().unread();
    let signed_out = app.state::<Inbox>().signed_out();
    let session = app.state::<Inbox>().token(&entry_id);
    let push_endpoint = app.state::<push::Push>().endpoint(&entry_id);
    let carried = webview::carried_state(app, &entry_id);
    // the floor this server is measured from, so the page can store it where the user's other
    // devices will find it. Unchanged by the visit — see `Inbox::baseline`.
    let read_floor = app.state::<Inbox>().baseline(&entry_id);

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

    // the page is this server's own client, signed in, so its session token is in reach — and it is
    // the only way Shiver can hold a connection to this server once the user has moved on
    inbox::harvest_token(app, entry_id);
}

/// Downloads the logos of servers added before Shiver inlined them.
///
/// The rail draws from `icon_data`, so without this an existing user's rail would be initials until
/// every server was re-added. It runs once at startup, off the main thread, and a failure leaves the
/// entry on its initials — an icon is decoration.
fn backfill_icons(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let missing = {
            let store = app.state::<Store>();
            let registry = store.registry();

            registry
                .servers
                .iter()
                .filter(|server| server.icon_data.is_none())
                .filter_map(|server| server.icon_url.clone().map(|url| (server.id.clone(), url)))
                .collect::<Vec<_>>()
        };

        for (id, url) in missing {
            let Some(data) = probe::fetch_icon(&url).await else {
                continue;
            };

            let store = app.state::<Store>();

            let _ = store.update(|registry| {
                if let Some(server) = registry.servers.iter_mut().find(|server| server.id == id) {
                    server.icon_data = Some(data.clone());
                }

                Ok(())
            });
        }
    });
}

pub fn open_externally(app: &tauri::AppHandle, url: &tauri::Url) {
    if !matches!(url.scheme(), "http" | "https") {
        return;
    }

    // said out loud: handing a page to the browser is the visible half of origin pinning, and when
    // it fires for something it should not, a launch turns into a browser tab with no explanation
    eprintln!("[shiver] {url} is outside this server, opening it in the browser");

    use tauri_plugin_opener::OpenerExt;

    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("[shiver] could not open {url} in the browser: {error}");
    }
}
