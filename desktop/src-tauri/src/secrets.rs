//! The session token and the password, in the OS keychain.
//!
//! Shiver signs in *for* the user so a server opens straight into the app rather than its login page,
//! which means Shiver does handle the password: it posts it to the server the user named and keeps it
//! here so it can sign in again later. Nothing is written to a file, the registry or Shiver's own json
//! — the keychain is what DESIGN.md's "never in plaintext" rule is satisfied by.
//!
//! Both are keyed by server entry id, never by origin, so two accounts on the same server stay
//! separate and neither can be read through the other.
//!
//! There is no backend here for Android or iOS. `keyring` has none, which is why the mobile client
//! keeps no credentials and signs in on the server's own page (`ARCHITECTURE.md` §3).

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

// android and ios have no keyring backend. rather than quietly falling back to a plaintext file,
// Shiver keeps nothing: the user signs in on the server's own page there until a platform keystore
// is wired up.
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
