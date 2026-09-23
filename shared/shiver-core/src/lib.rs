//! What both Shiver clients share: origin rules, the registry store, http, sign-in and probing.

pub mod error;
pub mod hash;
pub mod http;
pub mod login;
pub mod model;
pub mod origin;
pub mod probe;
pub mod store;

pub use error::{Error, Result};
pub use origin::{is_same_origin, normalize_origin};
pub use store::{LockExt, Store};
