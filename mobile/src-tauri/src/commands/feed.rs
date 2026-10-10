//! Unread counts and the DM list.

use tauri::State;

use crate::{
    inbox::{self, Inbox},
    store::Store,
};

#[tauri::command]
pub fn unread_counts(inbox: State<'_, Inbox>) -> std::collections::HashMap<String, u32> {
    inbox.unread()
}

/// Every server's conversations. Only Shiver's own pages can call this, so no server learns who the
/// user talks to on the others.
#[tauri::command]
pub fn list_dms(store: State<'_, Store>, inbox: State<'_, Inbox>) -> Vec<inbox::DmEntry> {
    inbox::collect_dms(&store.registry().servers, &inbox.dms(), &inbox.dm_times())
}
