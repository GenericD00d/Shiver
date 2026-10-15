//! Opening a server, once it may take another join.

use tauri::{AppHandle, Manager, State};

use shiver_core::limit::Joins;

use crate::{
    error::Result,
    inbox,
    store::{RegistryStore, Store},
    webview::{self, Showing},
};

use super::entry_of;

/// Tells Shiver's page a server opens only after a wait: `{ entryId, seconds }`.
const JOIN_WAIT_EVENT: &str = "shiver://join-wait";

/// Hands the webview to a server's client; `dm_user` asks the bridge to open that conversation once
/// connected.
///
/// Each page load joins the server, which Sharkord allows only a few times a minute, and past that
/// its client drops the session. So a load waits for its place (`limit::Joins`), with Shiver's page
/// counting down meanwhile; a newer choice made while it waits wins.
#[tauri::command]
pub async fn select_server(
    app: AppHandle,
    store: State<'_, Store>,
    id: String,
    dm_user: Option<String>,
) -> Result<()> {
    let entry = entry_of(&store, &id)?;
    let ticket = app.state::<Showing>().begin_opening();
    let token = inbox::session_for(&app, &id).await;
    let place = app.state::<Joins>().take(&entry.origin);

    if !place.wait.is_zero() {
        webview::emit_home(
            &app,
            JOIN_WAIT_EVENT,
            serde_json::json!({ "entryId": id, "seconds": place.wait.as_secs_f64().ceil() }),
        );
        tokio::time::sleep(place.wait).await;
    }

    if !app.state::<Showing>().still_opening(ticket) {
        app.state::<Joins>().give_back(&entry.origin, place);

        return Ok(());
    }

    // or the one the screens over the last page asked for on the way here
    let dm_user = dm_user.or_else(|| app.state::<Showing>().take_queued_dm(&id));

    app.state::<Showing>().set_pending_dm_user(dm_user);

    webview::show_server(&app, &entry, token.as_deref())?;
    inbox::sync(&app);

    store.update(|registry| {
        registry.settings.last_server_id = Some(id);

        Ok(())
    })
}
