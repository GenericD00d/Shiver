//! Connection errors. `TooLarge` and `Refused` are the two a caller must tell apart; everything
//! else is a server that could not be reached. Messages are user-facing and carry no secrets.

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidOrigin(String),

    #[error("Could not reach {0}. Check the address and that the server is running.")]
    Unreachable(String),

    #[error("{0} does not look like a Sharkord server")]
    NotSharkord(String),

    /// A size limit was hit; retrying gets the same oversized answer.
    #[error("{} in one message, and Shiver accepts {}", crate::readable_size(*size), crate::readable_size(*max))]
    TooLarge { size: usize, max: usize },

    /// The server said no (an expired session looks like this); the message is its own.
    #[error("{0}")]
    Refused(String),
}

/// For the add-server check: the same kind, and the same words, in the shared error.
impl From<Error> for shiver_core::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::InvalidOrigin(message) => Self::InvalidOrigin(message),
            Error::Unreachable(detail) => Self::Unreachable(detail),
            Error::NotSharkord(origin) => Self::NotSharkord(origin),
            Error::Refused(message) => Self::Refused(message),
            too_large @ Error::TooLarge { .. } => Self::Refused(too_large.to_string()),
        }
    }
}
