//! The pieces of Shiver that both clients have to agree on.
//!
//! Everything here existed twice — once in `desktop/src-tauri/src/`, once in `mobile/src-tauri/src/`
//! — and in the case of `normalize_origin` and `is_same_origin`, byte for byte, tests included.
//! `shared/sharkord-client` already makes this argument for the wire protocol: *"a second copy of
//! it would start drifting the day one client learned something the other did not"*. It applies at
//! least as strongly to the comparison that decides whether a webview may navigate somewhere.

pub mod error;
pub mod http;
pub mod origin;
pub mod store;

pub use error::{Error, Result};
pub use http::{client, json_within_limit, MAX_BODY};
pub use origin::{is_same_origin, normalize_origin};
pub use store::Store;
