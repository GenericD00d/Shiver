//! Shiver's screens over the server page: the quick rail's page (`tauri-plugin-shiver-rail`) also
//! draws a tile's menu, the settings and the direct messages, so reaching them does not unload the
//! page behind, nor drop its connection or its call.
//!
//! That page has no IPC. Its calls arrive by command name through the rail's channel and run here,
//! the same commands Shiver's own page has, but only those listed in `run`: nothing that opens a
//! server or adds one, which go through Shiver's page as before. Answers and the core's events go
//! back through [`tauri_plugin_shiver_rail::Rail::post`].

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tauri_plugin_shiver_rail::RailExt;
use zeroize::Zeroizing;

use shiver_core::model::NotifyLevel;

use crate::{commands, model::Settings, rail, update, webview};

/// Runs one call and posts its answer, `{ reply: id, ok, value }` (the value an error's message
/// when not ok). A change the rail shows redraws it.
pub fn call(app: &AppHandle, id: u32, command: String, args: Value) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        let answer = run(&app, &command, args).await;
        let ok = answer.is_ok();
        let value = answer.unwrap_or_else(Value::String);

        post(&app, json!({ "reply": id, "ok": ok, "value": value }));

        if ok && changes_rail(&command) {
            rail::refresh(&app);
        }
    });
}

/// Hands the page one of the core's events while it is up; it re-reads everything when it next opens.
pub fn emit(app: &AppHandle, event: &str, payload: &impl Serialize) {
    if rail::is_open(app) {
        post(app, json!({ "event": event, "payload": payload }));
    }
}

fn post(app: &AppHandle, message: Value) {
    let app = app.clone();

    // waits on the UI thread, which may be the caller's
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = app.shiver_rail().post(&message) {
            eprintln!("[shiver] could not answer the screens over the page: {error}");
        }
    });
}

/// What the rail draws from: names, logos, folders, the order and the colours.
fn changes_rail(command: &str) -> bool {
    matches!(
        command,
        "update_settings"
            | "refresh_server_info"
            | "remove_server"
            | "reorder_rail"
            | "reorder_servers"
            | "create_folder_with"
            | "set_server_folder"
            | "rename_folder"
            | "delete_folder"
            | "set_folder_expanded"
    )
}

fn read<T: DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|_| "Shiver could not read that request".to_string())
}

fn answer<T: Serialize>(result: crate::error::Result<T>) -> Result<Value, String> {
    let value = result.map_err(|error| error.to_string())?;

    serde_json::to_value(value).map_err(|error| error.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Id {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignIn {
    id: String,
    identity: String,
    password: Zeroizing<String>,
    remember_password: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Switch {
    id: String,
    accept: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Level {
    id: String,
    level: NotifyLevel,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushServer {
    entry_id: String,
    wanted: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Distributor {
    distributor: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Version {
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewSettings {
    settings: Settings,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Order {
    ordered: Vec<shiver_core::rail::RailRef>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderInFolder {
    ordered_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewFolder {
    name: String,
    member_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Move {
    id: String,
    folder_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rename {
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expand {
    id: String,
    expanded: bool,
}

/// The commands the screens over the page may call, by the names Shiver's own page calls them.
async fn run(app: &AppHandle, command: &str, args: Value) -> Result<Value, String> {
    let app = app.clone();

    match command {
        "list_registry" => answer(Ok(commands::list_registry(app.state()))),
        "server_icons" => answer(Ok(commands::server_icons(app.clone(), app.state()))),
        "session_states" => answer(Ok(commands::session_states(app.state(), app.state()))),
        "unread_counts" => answer(Ok(commands::unread_counts(app.state()))),
        "list_dms" => answer(Ok(commands::list_dms(app.state(), app.state()))),
        "app_version" => answer(Ok(commands::app_version())),
        "update_available" => answer(Ok(update::update_available(app.clone()))),
        "check_for_update" => answer(update::check_for_update(app.clone()).await),
        "skip_update" => {
            let Version { version } = read(args)?;

            answer(update::skip_update(app.clone(), version).await)
        }
        "open_repository" => answer(update::open_repository(app.clone())),
        "open_releases" => answer(update::open_releases(app.clone())),
        "update_settings" => {
            let NewSettings { settings } = read(args)?;

            answer(commands::update_settings(app.clone(), app.state(), settings).await)
        }
        "forget_trusted_links" => answer(commands::forget_trusted_links(app.state()).await),
        "forget_sessions" => {
            commands::forget_sessions(app.clone(), app.state());

            answer(Ok(()))
        }
        "push_status" => answer(Ok(commands::push_status(app.clone(), app.state()))),
        "set_push_server" => {
            let PushServer { entry_id, wanted } = read(args)?;

            answer(commands::set_push_server(app.clone(), entry_id, wanted).await)
        }
        "set_push_distributor" => {
            let Distributor { distributor } = read(args)?;

            answer(commands::set_push_distributor(app.clone(), distributor))
        }
        "set_accept_any_size" => {
            let Switch { id, accept } = read(args)?;

            answer(commands::set_accept_any_size(app.clone(), app.state(), id, accept).await)
        }
        "set_notify_level" => {
            let Level { id, level } = read(args)?;

            answer(commands::set_notify_level(app.state(), id, level).await)
        }
        "refresh_server_info" => {
            let Id { id } = read(args)?;

            answer(commands::refresh_server_info(app.clone(), app.state(), id).await)
        }
        "forget_password" => {
            let Id { id } = read(args)?;

            commands::forget_password(app.clone(), id);

            answer(Ok(()))
        }
        "sign_in_server" => {
            let SignIn {
                id,
                identity,
                password,
                remember_password,
            } = read(args)?;

            answer(
                commands::sign_in_server(
                    app.clone(),
                    app.state(),
                    id,
                    identity,
                    password,
                    remember_password,
                )
                .await,
            )
        }
        // the page behind goes too, if it was this server's: home, which opens another
        "remove_server" => {
            let Id { id } = read(args)?;
            let behind = on_screen(&app, &id);
            let result = answer(commands::remove_server(app.clone(), app.state(), id).await);

            if result.is_ok() && behind {
                webview::go_home(&app, None);
            }

            result
        }
        // the page behind was signed in as the account just logged out of: opened again, at the
        // server's own sign-in
        "log_out_server" => {
            let Id { id } = read(args)?;
            let behind = on_screen(&app, &id);
            let result =
                answer(commands::log_out_server(app.clone(), app.state(), id.clone()).await);

            if result.is_ok() && behind {
                webview::go_home(&app, Some(&format!("open={id}")));
            }

            result
        }
        "reorder_rail" => {
            let Order { ordered } = read(args)?;

            answer(commands::reorder_rail(app.state(), ordered).await)
        }
        "reorder_servers" => {
            let OrderInFolder { ordered_ids } = read(args)?;

            answer(commands::reorder_servers(app.state(), ordered_ids).await)
        }
        "create_folder_with" => {
            let NewFolder { name, member_ids } = read(args)?;

            answer(commands::create_folder_with(app.state(), name, member_ids).await)
        }
        "set_server_folder" => {
            let Move { id, folder_id } = read(args)?;

            answer(commands::set_server_folder(app.state(), id, folder_id).await)
        }
        "rename_folder" => {
            let Rename { id, name } = read(args)?;

            answer(commands::rename_folder(app.state(), id, name).await)
        }
        "delete_folder" => {
            let Id { id } = read(args)?;

            answer(commands::delete_folder(app.state(), id).await)
        }
        "set_folder_expanded" => {
            let Expand { id, expanded } = read(args)?;

            answer(commands::set_folder_expanded(app.state(), id, expanded).await)
        }
        _ => Err(format!("{command} is not available over a server's page")),
    }
}

fn on_screen(app: &AppHandle, entry_id: &str) -> bool {
    app.state::<webview::Showing>().server().as_deref() == Some(entry_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_read_as_shivers_page_sends_them() {
        let sign_in: SignIn = read(json!({
            "id": "a",
            "identity": "ana",
            "password": "secret",
            "rememberPassword": true
        }))
        .unwrap();

        assert_eq!(sign_in.id, "a");
        assert_eq!(sign_in.remember_password, Some(true));
        assert!(read::<Move>(json!({ "id": "a", "folderId": null }))
            .unwrap()
            .folder_id
            .is_none());
        assert!(read::<PushServer>(json!({ "entryId": "a" })).is_err());
        assert!(read::<Id>(json!({ "id": 3 })).is_err());
    }

    #[test]
    fn the_rail_redraws_only_for_what_it_shows() {
        assert!(changes_rail("rename_folder"));
        assert!(changes_rail("update_settings"));
        assert!(!changes_rail("list_dms"));
        assert!(!changes_rail("set_notify_level"));
    }
}
