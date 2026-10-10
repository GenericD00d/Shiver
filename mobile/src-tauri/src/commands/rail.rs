//! The rail's order and folders.

use tauri::State;
use uuid::Uuid;

use crate::{
    error::Result,
    model::Folder,
    store::{RegistryStore, Store},
};

/// Applies a drag at the top level (servers and folders share one position space).
#[tauri::command]
pub async fn reorder_rail(
    store: State<'_, Store>,
    ordered: Vec<shiver_core::rail::RailRef>,
) -> Result<()> {
    store.update(|registry| Ok(registry.rail().reorder(&ordered)?))
}

/// Numbers servers in the given order, within the folder they share (a drag inside a folder).
#[tauri::command]
pub async fn reorder_servers(store: State<'_, Store>, ordered_ids: Vec<String>) -> Result<()> {
    store.update(|registry| Ok(registry.rail().reorder_servers(&ordered_ids)?))
}

/// Creates a folder holding `member_ids`, at the first member's place, in one step.
#[tauri::command]
pub async fn create_folder_with(
    store: State<'_, Store>,
    name: String,
    member_ids: Vec<String>,
) -> Result<Folder> {
    store.update(|registry| {
        Ok(registry
            .rail()
            .create_folder(Uuid::new_v4().to_string(), &name, &member_ids)?)
    })
}

/// Moves a server into a folder (or out, with `None`); a folder left with one server dissolves.
#[tauri::command]
pub async fn set_server_folder(
    store: State<'_, Store>,
    id: String,
    folder_id: Option<String>,
) -> Result<()> {
    store.update(|registry| Ok(registry.rail().set_server_folder(&id, folder_id)?))
}

#[tauri::command]
pub async fn rename_folder(store: State<'_, Store>, id: String, name: String) -> Result<()> {
    store.update(|registry| Ok(registry.rail().rename_folder(&id, &name)?))
}

/// Deletes a folder; its servers move back to the top level.
#[tauri::command]
pub async fn delete_folder(store: State<'_, Store>, id: String) -> Result<()> {
    store.update(|registry| Ok(registry.rail().delete_folder(&id)?))
}

#[tauri::command]
pub async fn set_folder_expanded(
    store: State<'_, Store>,
    id: String,
    expanded: bool,
) -> Result<()> {
    store.update(|registry| Ok(registry.rail().set_folder_expanded(&id, expanded)?))
}
