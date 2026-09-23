use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, Error>;

/// Errors that reach the frontend. Messages are user-facing and never carry a token, password or path.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidOrigin(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("That server is not in your list")]
    UnknownServer,

    #[error("That folder is not in your list")]
    UnknownFolder,

    #[error("Could not reach {0}. Check the address and that the server is running.")]
    Unreachable(String),

    #[error("{0} does not look like a Sharkord server")]
    NotSharkord(String),

    #[error("Could not save your servers: {0}")]
    Storage(String),

    #[error("{0}")]
    Webview(String),

    /// The server said no; the message is its own.
    #[error("{0}")]
    Refused(String),
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<shiver_core::Error> for Error {
    fn from(value: shiver_core::Error) -> Self {
        use shiver_core::Error as Core;

        match value {
            Core::InvalidOrigin(message) => Error::InvalidOrigin(message),
            Core::Storage(message) => Error::Storage(message),
            Core::Unreachable(message) => Error::Unreachable(message),
            Core::NotSharkord(message) => Error::NotSharkord(message),
            Core::Refused(message) => Error::Refused(message),
            Core::InvalidInput(message) => Error::InvalidInput(message),
            Core::UnknownServer => Error::UnknownServer,
            Core::UnknownFolder => Error::UnknownFolder,
        }
    }
}

impl From<tauri::Error> for Error {
    fn from(value: tauri::Error) -> Self {
        Error::Webview(value.to_string())
    }
}
