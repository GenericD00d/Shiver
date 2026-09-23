//! What both Shiver clients share: origin rules, the registry store, the rail, http, sign-in and
//! probing.

pub mod error;
pub mod hash;
pub mod http;
pub mod limit;
pub mod login;
pub mod model;
pub mod origin;
pub mod probe;
pub mod rail;
pub mod store;

pub use error::{Error, Result};
pub use origin::{is_same_origin, normalize_origin};
pub use store::{LockExt, Store};
