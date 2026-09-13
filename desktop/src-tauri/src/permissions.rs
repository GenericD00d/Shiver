//! Forgetting a camera or microphone answer, so an accidental "block" is not for ever.
//!
//! WebView2 remembers what a site was allowed per origin, in the profile, and there is no way back
//! from inside the page: a user who pressed Block once is refused silently every time after, with
//! Sharkord showing only that the camera did not start. Chrome has an address-bar control for this;
//! a Tauri webview has no address bar, so Shiver has to offer the way back itself.
//!
//! **This used to call `ClearBrowsingData(SETTINGS)`, and that does not touch permissions.** The
//! button reported success and changed nothing, which was worse than not having it. The grants were
//! still sitting in the profile's `Preferences` afterwards, under `media_stream_mic` and
//! `media_stream_camera` with `setting: 1` — which is how it was finally proven rather than argued
//! about. `ICoreWebView2Profile4::SetPermissionState` is the API that actually owns them: setting a
//! permission back to `DEFAULT` is precisely "ask me next time".
//!
//! It is also per origin rather than per profile, so Shiver can now say how many answers it forgot
//! instead of claiming to have done something. A button that says "Forgotten" having forgotten
//! nothing is the failure this file exists to stop repeating.
//!
//! Screen sharing has no entry here, and that is not an oversight: WebView2 has no permission kind
//! for display capture, because Chromium never stores an answer for it. `getDisplayMedia` puts up
//! its picker every time and a refusal is only a refusal of that one attempt.

use tauri::{AppHandle, Manager};

use crate::error::{Error, Result};

/// Clears the camera and microphone answers WebView2 has stored, for every origin that has one.
///
/// Returns how many were forgotten, which the settings panel reports: nothing stored and nothing
/// cleared are the same button press, and the user is entitled to know which one happened.
#[cfg(windows)]
pub fn clear_media_permissions(app: &AppHandle) -> Result<usize> {
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
    use windows_core::Interface;

    // Any webview will do: they share one profile, which is the thing holding the answers. Shiver
    // configures no per-webview data directory, so there is exactly one. The shell is Shiver's own
    // page and is always there, unlike a server's.
    let webview = app
        .get_webview(crate::webviews::SHELL_WEBVIEW)
        .ok_or_else(|| Error::Webview("Shiver's own window is not open".into()))?;

    // The answer comes back from a completion handler on the webview's own thread, so it travels by
    // channel. `Mutex` only because the handler closures must be `Fn` rather than `FnOnce`.
    let (sender, receiver) = std::sync::mpsc::channel::<std::result::Result<usize, String>>();
    let sender = Arc::new(Mutex::new(Some(sender)));

    let answer = {
        let sender = sender.clone();

        move |outcome: std::result::Result<usize, String>| {
            if let Some(sender) = sender.lock().ok().and_then(|mut held| held.take()) {
                let _: std::result::Result<(), std::sync::mpsc::SendError<_>> = sender.send(outcome);
            }
        }
    };

    let media = |kind: COREWEBVIEW2_PERMISSION_KIND| {
        kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA || kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
    };

    webview
        .with_webview(move |platform| {
            let started = (|| unsafe {
                let core = platform
                    .controller()
                    .CoreWebView2()
                    .map_err(|error| error.to_string())?;

                // `Profile` arrived in ICoreWebView2_13 and `SetPermissionState` in Profile4; an
                // older runtime simply does not have them, and saying so is better than a button
                // that quietly does nothing — which is the whole history of this file.
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

                                // Collected first, because the count of things to forget has to be
                                // known before any of them is forgotten: the last completion is
                                // what reports, and "last" is meaningless without a total.
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

                                        targets.push((kind, origin.to_string().unwrap_or_default()));
                                    }
                                }

                                if targets.is_empty() {
                                    answer(Ok(0));

                                    return Ok(());
                                }

                                let total = targets.len();
                                let done = Arc::new(AtomicUsize::new(0));
                                let failed = Arc::new(Mutex::new(None::<String>));

                                // One slot per call, closed exactly once whether the call
                                // completed or never started. Written once and shared, because the
                                // two places that close a slot got out of step when they were not.
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

                                        // the last one home reports for all of them
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

                                    let handler =
                                        SetPermissionStateCompletedHandler::create(Box::new(
                                            move |result| {
                                                completed(result.err().map(|e| e.to_string()));

                                                Ok(())
                                            },
                                        ));

                                    let issued = {
                                        reset.SetPermissionState(
                                            kind,
                                            &wide,
                                            COREWEBVIEW2_PERMISSION_STATE_DEFAULT,
                                            &handler,
                                        )
                                    };

                                    // A call that never started has no completion coming, so its
                                    // slot is closed here — otherwise the tally never reaches the
                                    // total and the user waits out the timeout for nothing.
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

            // Only the failure to *start* is reported from here. Success is the handler's to
            // report, and reporting it twice would answer with a count nobody counted.
            if let Err(reason) = started {
                answer(Err(reason));
            }
        })
        .map_err(|error| Error::Webview(error.to_string()))?;

    // `with_webview` hands the closure to the webview's own thread and the completions arrive
    // there too, so the answer comes back by channel rather than by return. This thread is not that
    // thread, so blocking here does not stop the completions from being delivered.
    match receiver.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok(count)) => Ok(count),
        Ok(Err(reason)) => Err(Error::Webview(reason)),
        Err(_) => Err(Error::Webview(
            "The webview did not answer in time".to_string(),
        )),
    }
}

/// Nothing to clear anywhere else: the tests build this crate for the host, which is not WebView2.
#[cfg(not(windows))]
pub fn clear_media_permissions(_app: &AppHandle) -> Result<usize> {
    Err(Error::Webview(
        "Resetting site permissions is a WebView2 thing, and this is not Windows".to_string(),
    ))
}
