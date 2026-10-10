//! Sessions and the secrets behind them: what is kept in the encrypted store (written by one
//! thread, in order), restored at launch, renewed from the stored password, read from a page, and
//! forgotten on log-out.

use std::collections::{HashMap, HashSet};

use shiver_core::jwt::{self, Freshness, Renewals};
use tauri::{AppHandle, Manager};
use tauri_plugin_shiver_secrets::SecretsExt;
use zeroize::Zeroizing;

use super::{publish, sync, Inbox};
use crate::{store::Store, webview};

const BASELINE_PREFIX: &str = "baseline:";

const PASSWORD_PREFIX: &str = "password:";

/// The longest session token kept; the page decides what it stores.
const MAX_TOKEN: usize = 8 * 1024;

/// Reads a page's session: Sharkord's persistent auto-login token (`kept:`, which the user asked to
/// keep, so Shiver stores it) or the live session only (`live:`, used for this run only). A bare
/// expression, because `eval_with_callback` drops the value of a function with a `try` in it.
const TOKEN_SCRIPT: &str = "localStorage.getItem('sharkord-auto-login-token') \
     ? 'kept:' + localStorage.getItem('sharkord-auto-login-token') \
     : (sessionStorage.getItem('sharkord-token') \
        ? 'live:' + sessionStorage.getItem('sharkord-token') : null)";

pub(super) fn baseline_store_key(entry_id: &str) -> String {
    format!("{BASELINE_PREFIX}{entry_id}")
}

fn password_store_key(entry_id: &str) -> String {
    format!("{PASSWORD_PREFIX}{entry_id}")
}

/// The entry a store key belongs to (a bare key is the entry's session).
fn entry_of(key: &str) -> &str {
    [PASSWORD_PREFIX, BASELINE_PREFIX]
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .unwrap_or(key)
}

/// A write (`Some`) or removal (`None`) of one store key. Values are sessions and passwords (and
/// floors), so they are wiped once written.
type StoreWrite = (String, Option<Zeroizing<String>>);

/// Queues a write or removal for the one store-writer thread, which keeps writes in order and off
/// the UI thread (each is a blocking call into the JVM).
pub(super) fn store_off_thread(app: &AppHandle, key: String, value: Option<Zeroizing<String>>) {
    static WRITER: std::sync::OnceLock<std::sync::mpsc::Sender<StoreWrite>> =
        std::sync::OnceLock::new();

    let sender = WRITER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<StoreWrite>();
        let handle = app.clone();

        std::thread::Builder::new()
            .name("shiver-store".into())
            .spawn(move || {
                for (key, value) in receiver {
                    let result = match value {
                        Some(value) => handle.shiver_secrets().set(&key, &value),
                        None => handle.shiver_secrets().remove(&key),
                    };

                    if let Err(error) = result {
                        eprintln!(
                            "[shiver] could not update the encrypted store for {}: {error}",
                            entry_of(&key)
                        );
                    }
                }
            })
            .expect("the store writer thread should start");

        sender
    });

    let _ = sender.send((key, value));
}

/// Loads sessions, passwords and floors at startup (off the main thread), drops
/// anything for entries no longer in the rail, then connects.
pub fn restore(app: &AppHandle) {
    let app = app.clone();

    std::thread::spawn(move || {
        let keys = match app.shiver_secrets().keys() {
            Ok(keys) => keys,
            Err(error) => return eprintln!("[shiver] could not read the encrypted store: {error}"),
        };

        let known: HashSet<String> = app
            .state::<Store>()
            .registry()
            .servers
            .iter()
            .map(|server| server.id.clone())
            .collect();
        let inbox = app.state::<Inbox>();

        for key in keys {
            let entry_id = entry_of(&key).to_string();

            if !known.contains(&entry_id) {
                let _ = app.shiver_secrets().remove(&key);

                continue;
            }

            let Ok(Some(value)) = app
                .shiver_secrets()
                .get(&key)
                .map(|value| value.map(Zeroizing::new))
            else {
                continue;
            };

            if key.starts_with(BASELINE_PREFIX) {
                match serde_json::from_str::<HashMap<i64, u32>>(&value) {
                    Ok(baseline) => inbox.with(|state| state.baselines.insert(entry_id, baseline)),
                    Err(_) => {
                        let _ = app.shiver_secrets().remove(&key);

                        None
                    }
                };
            } else if key.starts_with(PASSWORD_PREFIX) {
                inbox.with(|state| state.remembered.insert(entry_id));
            } else {
                inbox.remember_token(&entry_id, &value);
            }
        }

        // Shiver opens the last server at launch, whose page joins it: the core's own connection
        // waiting out the grace saves a second join, which Sharkord allows only a few of a minute
        let last = app
            .state::<Store>()
            .registry()
            .settings
            .last_server_id
            .clone();

        if let Some(last) = last {
            app.state::<webview::Showing>().hold_back(&last);
        }

        sync(&app);
        sign_in_missing(&app);
    });
}

/// Signs in the servers that have a stored password but no session.
fn sign_in_missing(app: &AppHandle) {
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let missing: Vec<String> = {
            let inbox = handle.state::<Inbox>();
            let store = handle.state::<Store>();
            let registry = store.registry();

            registry
                .servers
                .iter()
                .filter(|server| server.identity.is_some())
                .filter(|server| {
                    inbox.token(&server.id).is_none() && inbox.has_password(&server.id)
                })
                .map(|server| server.id.clone())
                .collect()
        };

        let mut signed_in = false;

        for entry_id in missing {
            signed_in |= renew(&handle, &entry_id).await.is_some();
        }

        if signed_in {
            sync(&handle);
        }
    });
}

/// Keeps a session (in memory, and in the store when `persist`); returns whether it was new. A
/// token past `MAX_TOKEN` is not kept.
fn keep_session(app: &AppHandle, entry_id: &str, token: &str, persist: bool) -> bool {
    if token.len() > MAX_TOKEN {
        return false;
    }

    if app
        .state::<Inbox>()
        .with(|state| state.signed_out.remove(entry_id))
    {
        publish(app);
    }

    if !app.state::<Inbox>().remember_token(entry_id, token) {
        return false;
    }

    if persist {
        store_off_thread(
            app,
            entry_id.to_string(),
            Some(Zeroizing::new(token.to_string())),
        );
    }

    true
}

/// Keeps a session a renewal brought back, unless the entry was logged out or forgotten while it
/// was signing in (which takes the stored password, so a kept password is the sign it still
/// stands). Checked, kept and queued for the store under one lock, so a forget that follows queues
/// its removal after this write. Returns whether it was kept.
fn keep_renewed(app: &AppHandle, entry_id: &str, token: &str) -> bool {
    if token.len() > MAX_TOKEN {
        return false;
    }

    let (kept, was_signed_out) = app.state::<Inbox>().with(|state| {
        if !state.remembered.contains(entry_id) {
            return (false, false);
        }

        state
            .tokens
            .insert(entry_id.to_string(), Zeroizing::new(token.to_string()));
        store_off_thread(
            app,
            entry_id.to_string(),
            Some(Zeroizing::new(token.to_string())),
        );

        (true, state.signed_out.remove(entry_id))
    });

    if was_signed_out {
        publish(app);
    }

    kept
}

/// Keeps a session and connects with it.
fn record_session(app: &AppHandle, entry_id: &str, token: &str, persist: bool) {
    if keep_session(app, entry_id, token, persist) {
        sync(app);
    }
}

pub fn remember_session(app: &AppHandle, entry_id: &str, token: &str) {
    record_session(app, entry_id, token, true);
}

pub fn remember_password(app: &AppHandle, entry_id: &str, password: &str) {
    app.state::<Inbox>().with(|state| {
        state.remembered.insert(entry_id.to_string());
        state.signed_out.remove(entry_id);
    });
    store_off_thread(
        app,
        password_store_key(entry_id),
        Some(Zeroizing::new(password.to_string())),
    );
}

pub fn forget_password(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>()
        .with(|state| state.remembered.remove(entry_id));
    store_off_thread(app, password_store_key(entry_id), None);
}

/// Drops the session (not the password) and marks the server as waiting for a sign-in.
pub(super) fn forget_session(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>().forget_token(entry_id);
    crate::notify::forget(app, entry_id);
    store_off_thread(app, entry_id.to_string(), None);

    if app
        .state::<Inbox>()
        .with(|state| state.signed_out.insert(entry_id.to_string()))
    {
        publish(app);
    }
}

/// Drops everything Shiver holds for an entry, in memory and in the store.
pub fn forget_everywhere(app: &AppHandle, entry_id: &str) {
    app.state::<Inbox>().forget(entry_id);
    crate::notify::clear(app, entry_id);

    for key in [
        entry_id.to_string(),
        password_store_key(entry_id),
        baseline_store_key(entry_id),
    ] {
        store_off_thread(app, key, None);
    }
}

/// The session to open this entry with. One due for renewal is used as it is and renewed in the
/// background; a missing or expired one is renewed first, and an expired one is still used when
/// that fails (the server's refusal then takes its usual course).
pub async fn session_for(app: &AppHandle, entry_id: &str) -> Option<String> {
    let token = app.state::<Inbox>().token(entry_id);

    match token.as_deref().map(jwt::freshness) {
        Some(Freshness::Fresh) => token,
        Some(Freshness::Due) => {
            renew_in_background(app, entry_id);

            token
        }
        Some(Freshness::Expired) | None => renew(app, entry_id).await.or(token),
    }
}

fn renew_in_background(app: &AppHandle, entry_id: &str) {
    if !app.state::<Inbox>().has_password(entry_id) || !app.state::<Renewals>().begin(entry_id) {
        return;
    }

    let (app, entry_id) = (app.clone(), entry_id.to_string());

    tauri::async_runtime::spawn(async move {
        if renew(&app, &entry_id).await.is_some() {
            app.state::<Renewals>().succeeded(&entry_id);
        }
    });
}

/// Signs a server in again from its stored password and keeps the session, without connecting
/// with it (the caller does). A refused password is forgotten.
pub(super) async fn renew(app: &AppHandle, entry_id: &str) -> Option<String> {
    let (identity, origin) = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .and_then(|server| Some((server.identity.clone()?, server.origin.clone())))?;

    if !app.state::<Inbox>().has_password(entry_id) {
        return None;
    }

    let handle = app.clone();
    let key = password_store_key(entry_id);
    let password =
        match tokio::task::spawn_blocking(move || handle.shiver_secrets().get(&key)).await {
            Ok(Ok(Some(password))) => Zeroizing::new(password),
            Ok(Ok(None)) => return None,
            Ok(Err(error)) => {
                eprintln!("[shiver] could not read the stored password for {origin}: {error}");

                return None;
            }
            Err(_) => return None,
        };

    match shiver_core::login::sign_in(&origin, &identity, &password).await {
        Ok(session) => keep_renewed(app, entry_id, &session).then_some(session),
        Err(error @ shiver_core::Error::Refused(_)) => {
            eprintln!("[shiver] {origin} refused Shiver's stored password: {error}");
            forget_password(app, entry_id);

            None
        }
        Err(error) => {
            eprintln!("[shiver] could not reach {origin} to sign in again: {error}");

            None
        }
    }
}

/// After a server page loads, reads its session so the core can watch it once the user moves on.
pub fn harvest_token(app: &AppHandle, entry_id: String) {
    let Ok(window) = webview::main_window(app) else {
        return;
    };

    let handle = app.clone();

    let _ = window.eval_with_callback(TOKEN_SCRIPT, move |raw| {
        let Ok(serde_json::Value::String(answer)) = serde_json::from_str::<serde_json::Value>(&raw)
        else {
            return;
        };

        let Some((token, persist)) = answer
            .strip_prefix("kept:")
            .map(|token| (token.to_string(), true))
            .or_else(|| {
                answer
                    .strip_prefix("live:")
                    .map(|token| (token.to_string(), false))
            })
            .filter(|(token, _)| !token.is_empty())
        else {
            return;
        };

        // a page seeded before a background renewal still holds the older session
        if handle
            .state::<Inbox>()
            .token(&entry_id)
            .is_some_and(|held| jwt::outlasts(&held, &token))
        {
            return;
        }

        // off this thread: storing calls into the JVM, which deadlocks from an eval callback
        let (handle, id) = (handle.clone(), entry_id.clone());

        std::thread::spawn(move || record_session(&handle, &id, &token, persist));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_keys_name_their_entry() {
        assert_eq!(entry_of("abc"), "abc");
        assert_eq!(entry_of(&password_store_key("abc")), "abc");
        assert_eq!(entry_of(&baseline_store_key("abc")), "abc");
    }
}
