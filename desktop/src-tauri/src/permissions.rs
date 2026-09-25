//! Camera and microphone for server pages.
//!
//! On Windows Shiver answers WebView2's requests itself: only the page on screen may ask, and it
//! gets them once the user says yes to that server in a native dialog (the page cannot draw over
//! it). The page holding the call may also have the microphone again while hidden, as a call can
//! ask anew (a reconnect, another device). The yes is kept on the entry until the user logs out of
//! the server, removes it or forgets every yes here. WebView2 is told to remember nothing, since an
//! answer it remembers can be given without asking, past this gate.
//!
//! Elsewhere the platform answers: macOS allows every request, WebKitGTK refuses them.

use tauri::{AppHandle, Manager};

use crate::{
    error::Result,
    store::{RegistryStore, Store},
};

#[cfg(windows)]
pub use webview2::gate;

/// Forgets every server's yes. Returns how many servers had one.
pub fn forget_consents(app: &AppHandle) -> Result<usize> {
    app.state::<Store>().update(|registry| {
        let mut forgotten = 0;

        for server in &mut registry.servers {
            forgotten += usize::from(std::mem::take(&mut server.media_allowed));
        }

        Ok(forgotten)
    })
}

/// Only WebView2 asks.
#[cfg(not(windows))]
pub fn gate(_app: &AppHandle, _webview: &tauri::Webview, _entry: &crate::model::ServerEntry) {}

#[cfg(windows)]
mod webview2 {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use tauri::{AppHandle, Manager};
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    use url::Url;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Deferral, ICoreWebView2PermissionRequestedEventArgs,
        ICoreWebView2PermissionRequestedEventArgs3, ICoreWebView2Profile4, ICoreWebView2_13,
        COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA,
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE,
        COREWEBVIEW2_PERMISSION_STATE_ALLOW, COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
        COREWEBVIEW2_PERMISSION_STATE_DENY,
    };
    use webview2_com::{
        GetNonDefaultPermissionSettingsCompletedHandler, PermissionRequestedEventHandler,
        SetPermissionStateCompletedHandler,
    };
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows_core::{Interface, PWSTR};

    use crate::{
        error::{Error, Result},
        model::{is_same_origin, ServerEntry},
        store::{RegistryStore, Store},
        voice::VoiceState,
        webviews::{self, ActiveServer},
    };

    type Request = (
        ICoreWebView2PermissionRequestedEventArgs,
        ICoreWebView2Deferral,
    );

    thread_local! {
        /// The entry being asked about and its requests, held until the user answers. WebView2
        /// raises and settles requests on the main thread, so they wait there.
        static WAITING: RefCell<Option<(String, Vec<Request>)>> = const { RefCell::new(None) };
    }

    /// Answers this page's camera and microphone requests, and forgets what its profile saved
    /// before (off this thread, as that waits on the main thread).
    pub fn gate(app: &AppHandle, webview: &tauri::Webview, entry: &ServerEntry) {
        let (app, entry_id, origin) = (app.clone(), entry.id.clone(), entry.origin.clone());

        let registered = webview.with_webview(move |platform| unsafe {
            let handler = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };

                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();

                args.PermissionKind(&mut kind)?;

                let camera = kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA;

                if !camera && kind != COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
                    return Ok(());
                }

                if let Ok(args) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
                    args.SetSavesInProfile(false)?;
                }

                let same_origin =
                    Url::parse(&requester(&args)).is_ok_and(|url| is_same_origin(&origin, &url));

                match verdict(&app, &entry_id, camera, same_origin) {
                    Some(allowed) => args.SetState(state(allowed)),
                    None => wait(&app, &entry_id, args),
                }
            }));
            let mut token = 0;

            if let Err(error) = platform
                .controller()
                .CoreWebView2()
                .and_then(|core| core.add_PermissionRequested(&handler, &mut token))
            {
                eprintln!("[shiver] could not guard the camera and microphone: {error}");
            }
        });

        if let Err(error) = registered {
            eprintln!("[shiver] could not guard the camera and microphone: {error}");
        }

        let webview = webview.clone();

        std::thread::spawn(move || {
            if let Err(error) = forget_saved_answers(&webview) {
                eprintln!("[shiver] could not forget saved camera and microphone answers: {error}");
            }
        });
    }

    /// `Some` answers at once; `None` asks the user.
    fn verdict(app: &AppHandle, entry_id: &str, camera: bool, same_origin: bool) -> Option<bool> {
        let on_screen = app.state::<ActiveServer>().is_on_screen(entry_id);
        let in_call = !camera && app.state::<VoiceState>().holder().as_deref() == Some(entry_id);
        let allowed = app
            .state::<Store>()
            .registry()
            .server(entry_id)
            .is_some_and(|server| server.media_allowed);

        match (same_origin && (on_screen || in_call), allowed) {
            (false, _) => Some(false),
            (true, true) => Some(true),
            (true, false) => (!on_screen).then_some(false),
        }
    }

    /// Holds the request until the user answers; one question at a time, so another server's
    /// request meanwhile is refused.
    unsafe fn wait(
        app: &AppHandle,
        entry_id: &str,
        args: ICoreWebView2PermissionRequestedEventArgs,
    ) -> windows_core::Result<()> {
        let asking = WAITING.with_borrow(|waiting| waiting.as_ref().map(|(id, _)| id == entry_id));

        if asking == Some(false) {
            return args.SetState(state(false));
        }

        let request = (args.clone(), args.GetDeferral()?);

        WAITING.with_borrow_mut(|waiting| {
            waiting
                .get_or_insert_with(|| (entry_id.to_string(), Vec::new()))
                .1
                .push(request)
        });

        if asking.is_none() {
            ask(app, entry_id);
        }

        Ok(())
    }

    fn ask(app: &AppHandle, entry_id: &str) {
        let server = app
            .state::<Store>()
            .registry()
            .server(entry_id)
            .map(|server| server.name.clone())
            .unwrap_or_default();
        let mut dialog = app
            .dialog()
            .message(format!(
                "Let {server} use your camera and microphone?\n\nShiver remembers a yes until you \
                 log out of the server or remove it."
            ))
            .title("Camera and microphone")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Allow".into(),
                "Don't allow".into(),
            ));

        if let Ok(window) = webviews::main_window(app) {
            dialog = dialog.parent(&window);
        }

        let (app, entry_id) = (app.clone(), entry_id.to_string());

        dialog.show(move |allowed| {
            if allowed {
                let _ = app.state::<Store>().update(|registry| {
                    if let Some(server) = registry.server_mut(&entry_id) {
                        server.media_allowed = true;
                    }

                    Ok(())
                });
            }

            let _ = app.run_on_main_thread(move || settle(allowed));
        });
    }

    fn settle(allowed: bool) {
        for (args, deferral) in WAITING
            .take()
            .map(|(_, requests)| requests)
            .unwrap_or_default()
        {
            unsafe {
                let _ = args.SetState(state(allowed));
                let _ = deferral.Complete();
            }
        }
    }

    fn state(allowed: bool) -> COREWEBVIEW2_PERMISSION_STATE {
        if allowed {
            COREWEBVIEW2_PERMISSION_STATE_ALLOW
        } else {
            COREWEBVIEW2_PERMISSION_STATE_DENY
        }
    }

    /// The origin asking, as WebView2 reports it.
    unsafe fn requester(args: &ICoreWebView2PermissionRequestedEventArgs) -> String {
        let mut uri = PWSTR::null();

        if args.Uri(&mut uri).is_err() || uri.is_null() {
            return String::new();
        }

        let text = uri.to_string().unwrap_or_default();

        CoTaskMemFree(Some(uri.0 as *const core::ffi::c_void));

        text
    }

    /// Forgets the camera and microphone answers the page's profile saved before this gate.
    ///
    /// WebView2 answers through completion handlers on the main thread, so this blocks its own
    /// (non-main) thread on a channel rather than the event loop.
    fn forget_saved_answers(webview: &tauri::Webview) -> Result<usize> {
        let (sender, receiver) = std::sync::mpsc::channel::<std::result::Result<usize, String>>();
        let sender = Arc::new(Mutex::new(Some(sender)));

        // answers once; the handler closures must be `Fn`
        let answer = {
            let sender = sender.clone();

            move |outcome: std::result::Result<usize, String>| {
                if let Some(sender) = sender.lock().ok().and_then(|mut held| held.take()) {
                    let _: std::result::Result<(), std::sync::mpsc::SendError<_>> =
                        sender.send(outcome);
                }
            }
        };

        let media = |kind: COREWEBVIEW2_PERMISSION_KIND| {
            kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA
                || kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
        };

        webview
            .with_webview(move |platform| {
                let started = (|| unsafe {
                    let core = platform
                        .controller()
                        .CoreWebView2()
                        .map_err(|error| error.to_string())?;

                    let versioned: ICoreWebView2_13 = core.cast().map_err(|_| {
                        "This version of the WebView2 runtime cannot reset site permissions"
                            .to_string()
                    })?;

                    let profile: ICoreWebView2Profile4 = versioned
                        .Profile()
                        .map_err(|error| error.to_string())?
                        .cast()
                        .map_err(|_| {
                            "This version of the WebView2 runtime cannot reset site permissions"
                                .to_string()
                        })?;

                    let reset = profile.clone();
                    let answer = answer.clone();

                    profile
                        .GetNonDefaultPermissionSettings(
                            &GetNonDefaultPermissionSettingsCompletedHandler::create(Box::new(
                                move |result, settings| {
                                    if let Err(error) = result {
                                        answer(Err(error.to_string()));

                                        return Ok(());
                                    }

                                    let Some(settings) = settings else {
                                        answer(Ok(0));

                                        return Ok(());
                                    };

                                    let mut targets = Vec::new();
                                    let mut count = 0u32;

                                    {
                                        if settings.Count(&mut count).is_err() {
                                            answer(Err(
                                                "Could not read the stored permissions".into()
                                            ));

                                            return Ok(());
                                        }

                                        for index in 0..count {
                                            let Ok(setting) = settings.GetValueAtIndex(index)
                                            else {
                                                continue;
                                            };

                                            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                                            let mut origin = windows_core::PWSTR::null();

                                            if setting.PermissionKind(&mut kind).is_err() {
                                                continue;
                                            }

                                            if !media(kind) {
                                                continue;
                                            }

                                            if setting.PermissionOrigin(&mut origin).is_err() {
                                                continue;
                                            }

                                            let text = origin.to_string().unwrap_or_default();

                                            if !origin.is_null() {
                                                CoTaskMemFree(Some(
                                                    origin.0 as *const core::ffi::c_void,
                                                ));
                                            }

                                            targets.push((kind, text));
                                        }
                                    }

                                    if targets.is_empty() {
                                        answer(Ok(0));

                                        return Ok(());
                                    }

                                    let total = targets.len();
                                    let done = Arc::new(AtomicUsize::new(0));
                                    let failed = Arc::new(Mutex::new(None::<String>));

                                    let settle = {
                                        let done = done.clone();
                                        let failed = failed.clone();
                                        let answer = answer.clone();

                                        move |error: Option<String>| {
                                            if let Some(error) = error {
                                                if let Ok(mut held) = failed.lock() {
                                                    held.get_or_insert(error);
                                                }
                                            }

                                            if done.fetch_add(1, Ordering::SeqCst) + 1 == total {
                                                let reason = failed
                                                    .lock()
                                                    .ok()
                                                    .and_then(|mut held| held.take());

                                                answer(match reason {
                                                    Some(reason) => Err(reason),
                                                    None => Ok(total),
                                                });
                                            }
                                        }
                                    };

                                    for (kind, origin) in targets {
                                        let wide = windows_core::HSTRING::from(origin);
                                        let completed = settle.clone();

                                        let handler = SetPermissionStateCompletedHandler::create(
                                            Box::new(move |result| {
                                                completed(result.err().map(|e| e.to_string()));

                                                Ok(())
                                            }),
                                        );

                                        let issued = {
                                            reset.SetPermissionState(
                                                kind,
                                                &wide,
                                                COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
                                                &handler,
                                            )
                                        };

                                        if let Err(error) = issued {
                                            settle(Some(error.to_string()));
                                        }
                                    }

                                    Ok(())
                                },
                            )),
                        )
                        .map_err(|error| error.to_string())?;

                    Ok::<(), String>(())
                })();

                if let Err(reason) = started {
                    answer(Err(reason));
                }
            })
            .map_err(|error| Error::Webview(error.to_string()))?;

        match receiver.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(count)) => Ok(count),
            Ok(Err(reason)) => Err(Error::Webview(reason)),
            Err(_) => Err(Error::Webview(
                "The webview did not answer in time".to_string(),
            )),
        }
    }
}
