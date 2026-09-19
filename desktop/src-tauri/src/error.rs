use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, Error>;

/// Errors that reach the frontend. Messages are user-facing, so they name what to do about the
/// problem and never leak a token, a password or a filesystem path.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidOrigin(String),

    #[error("That server is not in your list")]
    UnknownServer,

    #[error("That folder is not in your list")]
    UnknownFolder,

    #[error("Could not reach {0}. Check the address and that the server is running.")]
    Unreachable(String),

    #[error("{0} does not look like a Sharkord server")]
    NotSharkord(String),

    #[error("{0}")]
    SignIn(String),

    #[error("Could not save your servers: {0}")]
    Storage(String),

    #[error("Could not reach the secure credential store: {0}")]
    Secrets(String),

    #[error("{0}")]
    Webview(String),
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// The shared crate raises only the two kinds it knows about, and each maps straight onto one of
/// this client's own.
impl From<shiver_core::Error> for Error {
    fn from(value: shiver_core::Error) -> Self {
        match value {
            shiver_core::Error::InvalidOrigin(message) => Error::InvalidOrigin(message),
            shiver_core::Error::Storage(message) => Error::Storage(message),
        }
    }
}

impl From<tauri::Error> for Error {
    fn from(value: tauri::Error) -> Self {
        Error::Webview(value.to_string())
    }
}
