//! Errors from the shared pieces. Messages are user-facing and never carry a token, password or path.

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidOrigin(String),

    #[error("{0}")]
    Storage(String),

    #[error("Could not reach {0}. Check the address and that the server is running.")]
    Unreachable(String),

    #[error("{0} does not look like a Sharkord server")]
    NotSharkord(String),

    /// The server answered and said no; the message is its own, made safe to display.
    #[error("{0}")]
    Refused(String),
}
