//! Waking a closed Shiver through UnifiedPush: which servers may, and the distributor.

use tauri::{AppHandle, Manager, State};
use tauri_plugin_shiver_push::PushExt;

use crate::{
    error::{Error, Result},
    store::Store,
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushStatus {
    /// package names of installed distributors
    pub distributors: Vec<String>,
    pub chosen: Option<String>,
    pub servers: Vec<PushServer>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushServer {
    pub id: String,
    pub name: String,
    pub wanted: bool,
    /// `off`, `waiting`, `ready` or `failed`
    pub state: &'static str,
}

/// What the notifications screen shows. No endpoint is returned, only whether one exists.
#[tauri::command]
pub fn push_status(app: AppHandle, store: State<'_, Store>) -> PushStatus {
    let (distributors, chosen) = app.shiver_push().distributors().unwrap_or_default();
    let push = app.state::<crate::push::Push>();
    let registry = store.registry();

    let servers = registry
        .servers
        .iter()
        .map(|server| {
            let wanted = registry.settings.push_servers.contains(&server.id);

            PushServer {
                id: server.id.clone(),
                name: server.name.clone(),
                wanted,
                state: if !wanted {
                    "off"
                } else if push.endpoint(&server.id).is_some() {
                    "ready"
                } else if push.has_failed(&server.id) {
                    "failed"
                } else {
                    "waiting"
                },
            }
        })
        .collect();

    PushStatus {
        distributors,
        chosen,
        servers,
    }
}

#[tauri::command]
pub async fn set_push_server(app: AppHandle, entry_id: String, wanted: bool) -> Result<()> {
    crate::push::set_wanted(&app, &entry_id, wanted)
}

/// Chooses a distributor and registers the chosen servers with it (answers arrive as events).
#[tauri::command]
pub fn set_push_distributor(app: AppHandle, distributor: String) -> Result<()> {
    crate::push::retire_endpoints(&app)?;

    app.shiver_push()
        .set_distributor(&distributor)
        .map_err(|error| Error::Webview(error.to_string()))?;

    crate::push::register_wanted(&app);

    Ok(())
}
