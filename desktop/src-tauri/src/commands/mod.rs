//! Commands called from Shiver's own webviews (the shell, bell and popup). Server pages have no IPC.
//!
//! Every command that writes the registry is `async`, so its disk write happens on a worker rather
//! than the UI thread. Passwords arrive as `Zeroizing<String>` so Shiver's copies are wiped.

use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use crate::{
    error::{Core, Result},
    model::{ServerEntry, Settings},
    store::Store,
    voice::VoiceState,
};

mod feed;
mod menu;
mod navigation;
mod popup;
mod rail;
mod servers;
mod settings;
mod voice;

pub use feed::*;
pub use menu::*;
pub use navigation::*;
pub use popup::*;
pub use rail::*;
pub use servers::*;
pub use settings::*;
pub use voice::*;

type Password = Zeroizing<String>;

fn entry_of(store: &Store, id: &str) -> Result<ServerEntry> {
    store
        .registry()
        .server(id)
        .cloned()
        .ok_or(Core::UnknownServer.into())
}

/// The entry, the settings and its mutes: what building a page needs.
fn page_inputs(store: &Store, id: &str) -> Result<(ServerEntry, Settings, Vec<i64>)> {
    let registry = store.registry();
    let entry = registry.server(id).cloned().ok_or(Core::UnknownServer)?;

    Ok((entry, registry.settings.clone(), registry.muted_for(id)))
}

fn voice_locked_for(app: &AppHandle, entry_id: &str) -> bool {
    app.state::<VoiceState>()
        .holder()
        .is_some_and(|holder| holder != entry_id)
}
