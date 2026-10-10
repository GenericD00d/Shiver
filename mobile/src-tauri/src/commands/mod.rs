//! Commands called from Shiver's own pages. Server pages have no IPC.
//!
//! Commands that write the registry are `async`, so the disk write happens off the UI thread.
//! Passwords arrive as `Zeroizing<String>` so Shiver's copies are wiped.

use zeroize::Zeroizing;

use crate::{
    error::{Core, Result},
    model::ServerEntry,
    store::Store,
};

mod feed;
mod navigation;
mod push;
mod rail;
mod servers;
mod settings;

pub use feed::*;
pub use navigation::*;
pub use push::*;
pub use rail::*;
pub use servers::*;
pub use settings::*;

type Password = Zeroizing<String>;

fn entry_of(store: &Store, id: &str) -> Result<ServerEntry> {
    store
        .registry()
        .server(id)
        .cloned()
        .ok_or(Core::UnknownServer.into())
}
