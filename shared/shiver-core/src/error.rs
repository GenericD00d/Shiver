//! What can go wrong in the pieces both clients share.
//!
//! Its own type rather than either client's: the two disagree about what else can fail, and neither
//! list is this crate's business. Both convert it into their own on the way out.
//!
//! Messages are user-facing. None of them carries a token, a password or a path.

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidOrigin(String),

    #[error("{0}")]
    Storage(String),
}
