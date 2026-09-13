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

    #[error("That folder is not in your rail")]
    UnknownFolder,

    #[error("Could not reach {0}. Check the address and that the server is running.")]
    Unreachable(String),

    #[error("{0} does not look like a Sharkord server")]
    NotSharkord(String),

    /// A server sent more in one message than Shiver accepts.
    ///
    /// Its own kind rather than an `Unreachable`, because it is the one connection failure that
    /// **will not** come right on its own: the next attempt asks the same question and gets the
    /// same oversized answer. Everything else Shiver retries quietly; this is worth telling someone
    /// about, because the server simply stops being watched.
    ///
    /// Both numbers are kept rather than a message, because they are what anyone reading this
    /// actually needs — how much the server sent, and what Shiver would have taken.
    #[error("{size} bytes in one message, and Shiver accepts {max}")]
    TooLarge { size: usize, max: usize },

    #[error("Could not save your servers: {0}")]
    Storage(String),

    #[error("{0}")]
    Webview(String),

    /// A server answered and said no. The message is the server's own words, already written for
    /// a person to read, so it is passed through rather than replaced.
    #[error("{0}")]
    Refused(String),
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<tauri::Error> for Error {
    fn from(value: tauri::Error) -> Self {
        Error::Webview(value.to_string())
    }
}
