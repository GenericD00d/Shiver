//! Resetting the camera and microphone answers WebView2 remembers per site, so an accidental
//! "Block" is not permanent (a webview has no address bar to undo it from).
//!
//! Each rail entry has its own browser profile, so each is reset through one of its own webviews.
//! Entries with no page open are reset when their page is next built (`apply_pending_reset`).

use tauri::{AppHandle, Manager};

use crate::{
    error::{Error, Result},
    store::{RegistryStore, Store},
    webviews,
};

/// Resets every entry's answers: open pages now, the rest when next opened. Returns how many
/// answers were forgotten now.
pub fn clear_media_permissions(app: &AppHandle) -> Result<usize> {
    let entries: Vec<String> = app
        .state::<Store>()
        .registry()
        .servers
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    let mut forgotten = 0;
    let mut later = Vec::new();

    for entry_id in entries {
        match page_of(app, &entry_id) {
            Some(webview) => forgotten += reset_webview(&webview)?,
            None => later.push(entry_id),
        }
    }

    app.state::<Store>().update(|registry| {
        registry.pending_permission_resets = later;

        Ok(())
    })?;

    Ok(forgotten)
}

/// If this entry is waiting for a reset, performs it (off this thread) now that it has a page.
pub fn apply_pending_reset(app: &AppHandle, entry_id: &str) {
    let pending = app
        .state::<Store>()
        .registry()
        .pending_permission_resets
        .iter()
        .any(|id| id == entry_id);

    if !pending {
        return;
    }

    let app = app.clone();
    let entry_id = entry_id.to_string();

    std::thread::spawn(move || {
        let Some(webview) = page_of(&app, &entry_id) else {
            return;
        };

        match reset_webview(&webview) {
            Ok(_) => {
                let _ = app.state::<Store>().update(|registry| {
                    registry
                        .pending_permission_resets
                        .retain(|id| id != &entry_id);

                    Ok(())
                });
            }
            Err(error) => eprintln!("[shiver] could not reset permissions for {entry_id}: {error}"),
        }
    });
}

fn page_of(app: &AppHandle, entry_id: &str) -> Option<tauri::Webview> {
    app.get_webview(&webviews::webview_label(entry_id))
        .or_else(|| app.get_webview(&webviews::dm_webview_label(entry_id)))
}

/// Resets camera and microphone answers in one webview's profile. Returns how many were forgotten.
///
/// WebView2 answers through completion handlers on the main thread, so this blocks its own
/// (non-main) thread on a channel rather than the event loop.
#[cfg(windows)]
fn reset_webview(webview: &tauri::Webview) -> Result<usize> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Profile4, ICoreWebView2_13, COREWEBVIEW2_PERMISSION_KIND,
        COREWEBVIEW2_PERMISSION_KIND_CAMERA, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
        COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
    };
    use webview2_com::{
        GetNonDefaultPermissionSettingsCompletedHandler, SetPermissionStateCompletedHandler,
    };
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows_core::Interface;

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
                    "This version of the WebView2 runtime cannot reset site permissions".to_string()
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
                                        answer(Err("Could not read the stored permissions".into()));

                                        return Ok(());
                                    }

                                    for index in 0..count {
                                        let Ok(setting) = settings.GetValueAtIndex(index) else {
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
                                            let reason =
                                                failed.lock().ok().and_then(|mut held| held.take());

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

/// WebView2 only.
#[cfg(not(windows))]
fn reset_webview(_webview: &tauri::Webview) -> Result<usize> {
    Err(Error::Webview(
        "Resetting site permissions is only supported with WebView2, on Windows".to_string(),
    ))
}
