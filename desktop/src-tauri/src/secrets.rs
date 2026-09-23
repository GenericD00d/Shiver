//! The session token and password, in the OS keychain, keyed by rail entry (never by origin).
//!
//! `keyring` blocks (on Linux it is a D-Bus call that may prompt), so async callers use the
//! `*_off_thread` versions. Values are held in `Zeroizing` so Shiver's copies are wiped on drop.

use zeroize::Zeroizing;

use crate::error::{Error, Result};

const SERVICE: &str = "com.shiver.client";

pub type SecretString = Zeroizing<String>;

#[derive(Clone, Copy)]
pub enum Secret {
    /// the session token from /login, good for seven days
    Session,
    /// kept so Shiver can sign in again when the session expires
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
    use super::*;

    fn entry(kind: Secret, entry_id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, &account_key(kind, entry_id))
            .map_err(|error| Error::Secrets(error.to_string()))
    }

    pub fn set(kind: Secret, entry_id: &str, value: &str) -> Result<()> {
        entry(kind, entry_id)?
            .set_password(value)
            .map_err(|error| Error::Secrets(error.to_string()))
    }

    pub fn get(kind: Secret, entry_id: &str) -> Result<Option<SecretString>> {
        match entry(kind, entry_id)?.get_password() {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
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

/// No keychain on this platform: storing fails loudly rather than pretending, and there is never
/// anything to read. (The Android client has its own Keystore-backed store.)
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod backend {
    use super::*;

    pub fn set(_kind: Secret, _entry_id: &str, _value: &str) -> Result<()> {
        Err(Error::Secrets(
            "this platform has no credential store Shiver can use".into(),
        ))
    }

    pub fn get(_kind: Secret, _entry_id: &str) -> Result<Option<SecretString>> {
        Ok(None)
    }

    pub fn delete(_kind: Secret, _entry_id: &str) -> Result<()> {
        Ok(())
    }
}

/// Deletes both secrets, attempting both even if the first fails.
fn forget_all(entry_id: &str) -> Result<()> {
    let session = backend::delete(Secret::Session, entry_id);
    let password = backend::delete(Secret::Password, entry_id);

    session.and(password)
}

async fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| Error::Secrets(error.to_string()))?
}

/// Reads a secret. A keychain failure is logged and read as absent.
pub async fn read_off_thread(kind: Secret, entry_id: &str) -> Option<SecretString> {
    let id = entry_id.to_string();

    off_thread(move || backend::get(kind, &id))
        .await
        .unwrap_or_else(|error| {
            eprintln!("[shiver] could not read a secret for {entry_id}: {error}");

            None
        })
}

pub async fn store_off_thread(kind: Secret, entry_id: &str, value: &str) -> Result<()> {
    let id = entry_id.to_string();
    let value = Zeroizing::new(value.to_string());

    off_thread(move || backend::set(kind, &id, &value)).await
}

pub async fn forget_off_thread(kind: Secret, entry_id: &str) -> Result<()> {
    let id = entry_id.to_string();

    off_thread(move || backend::delete(kind, &id)).await
}

pub async fn forget_all_off_thread(entry_id: &str) -> Result<()> {
    let id = entry_id.to_string();

    off_thread(move || forget_all(&id)).await
}
