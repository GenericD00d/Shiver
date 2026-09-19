//! The session token and the password, in the OS keychain.
//!
//! Shiver signs in *for* the user so a server opens straight into the app rather than its login page,
//! which means Shiver does handle the password: it posts it to the server the user named and keeps it
//! here so it can sign in again later. Nothing is written to a file, the registry or Shiver's own json
//! — the keychain is what the "never in plaintext" rule in the README is satisfied by.
//!
//! Both are keyed by server entry id, never by origin, so two accounts on the same server stay
//! separate and neither can be read through the other.
//!
//! There is no `keyring` backend for Android or iOS, so this module has none either. That does not
//! mean the mobile client keeps nothing — it brings its own store rather than going without:
//! `mobile/plugins/tauri-plugin-shiver-secrets` holds the same two secrets in
//! `EncryptedSharedPreferences`, keyed by the Android Keystore. This file is the desktop half of
//! that arrangement, not the whole of it.

use crate::error::Result;

const SERVICE: &str = "com.shiver.client";

/// What a secret is for. Both are keyed by rail entry, never by origin, so two accounts on one
/// server cannot read each other.
#[derive(Clone, Copy)]
pub enum Secret {
    /// the session token from /login, good for seven days
    Session,
    /// kept so Shiver can sign in again when that token expires, which is what makes opening a
    /// server seamless rather than a weekly trip through the login page
    Password,
}

fn account_key(kind: Secret, entry_id: &str) -> String {
    match kind {
        Secret::Session => format!("session:{entry_id}"),
        Secret::Password => format!("password:{entry_id}"),
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
mod backend {
    use super::{account_key, Secret, SERVICE};
    use crate::error::{Error, Result};

    fn entry(kind: Secret, entry_id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, &account_key(kind, entry_id))
            .map_err(|error| Error::Secrets(error.to_string()))
    }

    pub fn set(kind: Secret, entry_id: &str, value: &str) -> Result<()> {
        entry(kind, entry_id)?
            .set_password(value)
            .map_err(|error| Error::Secrets(error.to_string()))
    }

    pub fn get(kind: Secret, entry_id: &str) -> Result<Option<String>> {
        match entry(kind, entry_id)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(Error::Secrets(error.to_string())),
        }
    }

    pub fn delete(kind: Secret, entry_id: &str) -> Result<()> {
        match entry(kind, entry_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(Error::Secrets(error.to_string())),
        }
    }
}

// No keyring backend here, and no plaintext fallback: a secret that cannot be stored safely is one
// this module declines to store. The mobile client does not reach this arm — it has its own
// Keystore-backed plugin — so anything that does land here is a platform Shiver has not been taught
// about, and losing the secret is the right failure.
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod backend {
    use super::Secret;
    use crate::error::Result;

    pub fn set(_kind: Secret, _entry_id: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    pub fn get(_kind: Secret, _entry_id: &str) -> Result<Option<String>> {
        Ok(None)
    }

    pub fn delete(_kind: Secret, _entry_id: &str) -> Result<()> {
        Ok(())
    }
}

pub fn store(kind: Secret, entry_id: &str, value: &str) -> Result<()> {
    backend::set(kind, entry_id, value)
}

pub fn read(kind: Secret, entry_id: &str) -> Result<Option<String>> {
    backend::get(kind, entry_id)
}

/// Drops one secret, keeping the other.
///
/// The password when the user declined to have it kept, which is the only case: a session with no
/// password behind it is the ordinary state of an opted-out server, and dropping a session while
/// keeping the password would just mean signing in again on the next open.
///
/// Called on every sign-in rather than only when the box changes, so unticking it on a server that
/// was added with it ticked actually removes what is already there.
pub fn forget(kind: Secret, entry_id: &str) -> Result<()> {
    backend::delete(kind, entry_id)
}

/// Called whenever a server leaves the rail, so Shiver keeps nothing about a server the user left.
pub fn forget_all(entry_id: &str) -> Result<()> {
    backend::delete(Secret::Session, entry_id)?;
    backend::delete(Secret::Password, entry_id)
}

/* ───────────────── off the async runtime's workers ───────────────── */

// Every function above is synchronous, and `keyring` genuinely blocks: on Linux a read is a D-Bus
// round trip to the Secret Service that can take a while and can put a keyring-unlock prompt in
// front of the user. Most of Shiver's callers are `async fn`s, and a bare call from one of those
// parks a runtime worker for the duration — the same hazard `permissions::clear_media_permissions`
// documents at length, applied to the calls that happen far more often.
//
// So the async callers use these instead, and the synchronous ones keep the plain versions.

/// One secret, read without blocking the caller's worker.
pub async fn read_off_thread(kind: Secret, entry_id: &str) -> Option<String> {
    let entry_id = entry_id.to_string();

    tauri::async_runtime::spawn_blocking(move || read(kind, &entry_id).ok().flatten())
        .await
        .ok()
        .flatten()
}

/// One secret, written without blocking the caller's worker.
pub async fn store_off_thread(kind: Secret, entry_id: &str, value: &str) -> Result<()> {
    let entry_id = entry_id.to_string();
    let value = value.to_string();

    tauri::async_runtime::spawn_blocking(move || store(kind, &entry_id, &value))
        .await
        .map_err(|error| crate::error::Error::Secrets(error.to_string()))?
}

/// One secret, dropped without blocking the caller's worker.
pub async fn forget_off_thread(kind: Secret, entry_id: &str) -> Result<()> {
    let entry_id = entry_id.to_string();

    tauri::async_runtime::spawn_blocking(move || forget(kind, &entry_id))
        .await
        .map_err(|error| crate::error::Error::Secrets(error.to_string()))?
}

/// Both of a server's secrets, dropped without blocking the caller's worker.
pub async fn forget_all_off_thread(entry_id: &str) -> Result<()> {
    let entry_id = entry_id.to_string();

    tauri::async_runtime::spawn_blocking(move || forget_all(&entry_id))
        .await
        .map_err(|error| crate::error::Error::Secrets(error.to_string()))?
}
