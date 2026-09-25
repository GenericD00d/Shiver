use serde::{Serialize, Serializer};

pub use shiver_core::Error as Core;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors that reach the frontend: the shared ones, and this platform's. Messages are user-facing
/// and never carry a token, password or path.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] Core),

    #[error("{0}")]
    Webview(String),
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
