//! What can go wrong talking to a server, in the terms this crate can actually distinguish.
//!
//! Its own type rather than either client's, because the two clients disagree about what else can
//! go wrong — desktop has a keychain to fail, mobile has a rail entry to not find — and neither
//! list is this crate's business. Both convert it into their own on the way out, which is also the
//! point at which the two variants below stop being shared: **`TooLarge` and `Refused` are the two
//! a caller must tell apart**, and every other failure here is a server that could not be reached.
//!
//! Messages are user-facing. None of them carries a token, a password or a path.

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    InvalidOrigin(String),

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

    /// A server answered and said no. The message is the server's own words, already written for
    /// a person to read, so it is passed through rather than replaced.
    ///
    /// This is what an expired session looks like, and it is the signal that makes Shiver sign in
    /// again — so a caller that folds it into a general connection failure will keep retrying a
    /// token that is never going to work.
    #[error("{0}")]
    Refused(String),
}
