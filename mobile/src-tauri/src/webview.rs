//! The one webview, and what it is showing.
//!
//! Android gives a window a single webview, so Shiver is a switcher: the webview shows either
//! Shiver's own pages or one server's client, and opening a server is a navigation. The server's
//! session travels in a `#shiver-seed=<key>.<token>` fragment that the document-start script takes
//! out of the URL before the page's scripts run and serves from memory (see `shared/web/session.ts`),
//! so it is never written to the webview's storage. The key is SHA-256 of a per-launch secret, which
//! lives only in that script's closure, and the page's origin, so one server's key is no use on
//! another. The bridge runs after load; anything it reads back out of a page is a request, never
//! trusted state.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use shiver_core::{rail::RailRef, LockExt};
use tauri::{window::Color, AppHandle, Manager, Url, WebviewWindow};

pub use shiver_core::limit::Openings;

use crate::{
    error::{Error, Result},
    inbox,
    model::{is_same_origin, Folder, ServerEntry, Settings, MAX_SOUND_VOLUME},
    store::{RegistryStore, Store},
};

pub const MAIN_WINDOW: &str = "main";

const BRIDGE_SOURCE: &str = include_str!("../generated/bridge.js");

/// Runs at document start in every page: takes the seeded session out of the URL and applies
/// Sharkord's light/dark class before first paint. Built from `mobile/bridge/document-start.ts`.
const DOCUMENT_START: &str = include_str!("../generated/document-start.js");

/// The fragment parameter `DOCUMENT_START` reads the session from (`SEED_PARAM` in shared/web).
const SEED_PARAM: &str = "shiver-seed";

fn seed_key() -> &'static str {
    static KEY: OnceLock<String> = OnceLock::new();

    KEY.get_or_init(|| uuid::Uuid::new_v4().simple().to_string())
}

fn seed_key_for(secret: &str, url: &Url) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{secret}{}", url.origin().ascii_serialization()))
    )
}

pub fn document_start() -> String {
    format!(
        "(function(SHIVER_SEED_KEY){{{DOCUMENT_START}\n}})(\"{}\");",
        seed_key()
    )
}

pub fn without_seed(url: &Url) -> Url {
    let mut url = url.clone();

    if url
        .fragment()
        .is_some_and(|fragment| fragment.contains(SEED_PARAM))
    {
        url.set_fragment(None);
    }

    url
}

/// Bounds on what one page's rail may ask of the registry per poll.
const MAX_CREATES_PER_POLL: usize = 5;
const MAX_FOLDER_ID: usize = 64;
const MAX_PAGE_STATE_BYTES: usize = 1024 * 1024;
const DEFAULT_FOLDER_NAME: &str = "Folder";

#[derive(Default)]
struct ShowingState {
    /// where Shiver's own pages are; first write wins (dev server and app URL differ)
    home: Option<String>,
    /// the entry on screen, if any
    server: Option<String>,
    loaded: bool,
    /// open the conversation with this user on arrival (consumed once)
    pending_dm_user: Option<String>,
}

/// Where Shiver's own pages live and which server the webview is on.
#[derive(Default)]
pub struct Showing(Mutex<ShowingState>);

impl Showing {
    pub fn set_home_if_unset(&self, url: String) {
        self.0.locked().home.get_or_insert(url);
    }

    pub fn home(&self) -> Option<String> {
        self.0.locked().home.clone()
    }

    pub fn set_server(&self, entry_id: Option<String>) {
        let mut state = self.0.locked();

        state.server = entry_id;
        state.loaded = false;
    }

    pub fn set_loaded(&self) {
        self.0.locked().loaded = true;
    }

    pub fn loading(&self) -> bool {
        let state = self.0.locked();

        state.server.is_some() && !state.loaded
    }

    pub fn server(&self) -> Option<String> {
        self.0.locked().server.clone()
    }

    pub fn set_pending_dm_user(&self, user: Option<String>) {
        self.0.locked().pending_dm_user = user;
    }

    pub fn take_pending_dm_user(&self) -> Option<String> {
        self.0.locked().pending_dm_user.take()
    }
}

pub fn main_window(app: &AppHandle) -> Result<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW)
        .ok_or_else(|| Error::Webview("The Shiver window is not open".into()))
}

/// Navigates to a server's client, seeding `token` through the URL fragment when there is one.
pub fn show_server(app: &AppHandle, entry: &ServerEntry, token: Option<&str>) -> Result<()> {
    let window = main_window(app)?;

    let mut url = Url::parse(&entry.origin)
        .map_err(|_| Error::InvalidOrigin(format!("'{}' is not a valid address", entry.origin)))?;

    if let Some(token) = token.filter(|token| !token.is_empty()) {
        let encoded: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();

        let key = seed_key_for(seed_key(), &url);

        url.set_fragment(Some(&format!("{SEED_PARAM}={key}.{encoded}")));
    }

    app.state::<Showing>().set_server(Some(entry.id.clone()));

    Ok(window.navigate(url)?)
}

/// Returns to Shiver's page saying the server on screen could not be opened.
pub fn show_failed(app: &AppHandle) {
    let showing = app.state::<Showing>();
    let (Some(id), Some(Ok(mut url))) = (
        showing.server(),
        showing.home().map(|home| Url::parse(&home)),
    ) else {
        return;
    };

    url.set_fragment(Some(&format!("failed={id}")));

    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        if let Ok(window) = main_window(&app) {
            let _ = window.navigate(url);
        }
    });
}

/// Everything one server page is handed when the bridge is installed in it.
pub struct PageContext<'a> {
    pub entry: &'a ServerEntry,
    pub settings: &'a Settings,
    /// channel ids muted on this entry
    pub muted: &'a [i64],
    /// every server, reduced by `rail_payload` before it reaches the page
    pub servers: &'a [ServerEntry],
    pub unread: &'a HashMap<String, u32>,
    /// entries waiting for the user to sign in again
    pub signed_out: &'a [String],
    /// this entry's own session, never another's (used by the bridge to reconnect)
    pub session: Option<&'a str>,
    /// the unread floor to share with the user's other devices through the companion plugin
    pub read_floor: Option<&'a HashMap<i64, u32>>,
    pub folders: &'a [Folder],
    /// this entry's own push endpoint, for the plugin's relay
    pub push_endpoint: Option<&'a str>,
    /// endpoints the plugin should forget
    pub retired_push_endpoints: &'a [String],
}

/// Evaluates the bridge in a server page that has just finished loading.
pub fn install_bridge(app: &AppHandle, page: PageContext<'_>) {
    let Ok(window) = main_window(app) else {
        return;
    };

    let showing = app.state::<Showing>();

    let Some(home) = showing.home() else {
        return;
    };

    let config = json!({
        "entryId": page.entry.id,
        "origin": page.entry.origin,
        "serverName": page.entry.name,
        "theme": theme_payload(page.settings),
        "muted": page.muted,
        "soundVolume": page.settings.sound_volume.min(MAX_SOUND_VOLUME),
        "minimiseAttachments": page.settings.minimise_attachments,
        // JSON keys are strings; read back by `parse_shared_floor` in sharkord-client
        "readFloor": page.read_floor.map(|floor| {
            floor.iter().map(|(channel, count)| (channel.to_string(), *count)).collect::<HashMap<_, _>>()
        }),
        // the page has no IPC, so going back is a navigation to here
        "home": home,
        "rail": rail_payload(page.servers, page.unread, page.signed_out),
        "folders": folder_payload(page.folders),
        "pushEndpoint": page.push_endpoint,
        "retiredPushEndpoints": page.retired_push_endpoints,
        "openDmUser": showing.take_pending_dm_user(),
        "session": page.session,
    });

    let _ = window.eval(format!("window.__SHIVER__ = {config};\n{BRIDGE_SOURCE}"));
}

/// The rail as a server's page may see it: display names, inlined logos, opaque entry ids,
/// positions, folder membership, unread counts and signed-out flags. Never an origin, icon URL,
/// account label or identity; tapping a tile navigates to Shiver's page with only the id.
fn rail_payload(
    servers: &[ServerEntry],
    unread: &HashMap<String, u32>,
    signed_out: &[String],
) -> Value {
    let mut ordered: Vec<&ServerEntry> = servers.iter().collect();

    ordered.sort_by_key(|server| server.position);

    ordered
        .into_iter()
        .map(|server| {
            json!({
                "id": server.id,
                "name": server.name,
                "icon": server.icon_data,
                "position": server.position,
                "folderId": server.folder_id,
                "unread": unread.get(&server.id).copied().unwrap_or(0),
                "signedOut": signed_out.contains(&server.id),
            })
        })
        .collect()
}

/// The user's folders (names they typed into Shiver), ordered.
fn folder_payload(folders: &[Folder]) -> Value {
    let mut ordered: Vec<&Folder> = folders.iter().collect();

    ordered.sort_by_key(|folder| folder.position);

    ordered
        .into_iter()
        .map(|folder| {
            json!({
                "id": folder.id,
                "name": folder.name,
                "position": folder.position,
                "expanded": folder.expanded,
            })
        })
        .collect()
}

/// Polled by `inbox::watch_mutes`: reads the page's mutes, rail changes and queued outside links
/// (a read on departure would be torn down by the navigation before it answered).
const READ_SCRIPT: &str = "JSON.stringify({ \
    muted: window.__SHIVER_MUTED__ ? window.__SHIVER_MUTED__() : null, \
    rail: window.__SHIVER_RAIL_STATE__ ? window.__SHIVER_RAIL_STATE__() : null, \
    open: window.__SHIVER_OPEN__ ? window.__SHIVER_OPEN__() : null })";

#[derive(Default, Deserialize)]
struct PageState {
    muted: Option<Vec<i64>>,
    rail: Option<RailState>,
    #[serde(default, deserialize_with = "null_as_default")]
    open: Vec<String>,
}

fn null_as_default<'de, D: serde::Deserializer<'de>, T: Default + Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

pub fn read_mutes(app: &AppHandle, entry_id: &str) {
    let Ok(window) = main_window(app) else {
        return;
    };

    let handle = app.clone();
    let entry_id = entry_id.to_string();

    let _ = window.eval_with_callback(READ_SCRIPT, move |raw| {
        // JSON-encoded twice: the result is a string holding the object
        if raw.len() > MAX_PAGE_STATE_BYTES {
            return;
        }

        let Ok(Value::String(body)) = serde_json::from_str::<Value>(&raw) else {
            return;
        };

        let Ok(state) = serde_json::from_str::<PageState>(&body) else {
            return;
        };

        let (app, entry_id) = (handle.clone(), entry_id.clone());

        // off the callback thread: what follows ends in an `eval`, which deadlocks from here
        std::thread::spawn(move || apply_page_state(&app, &entry_id, state));
    });
}

fn apply_page_state(app: &AppHandle, entry_id: &str, state: PageState) {
    // what the user opened away from the page, queued by the document-start script
    for url in app
        .state::<Openings>()
        .grant(entry_id, &state.open)
        .iter()
        .filter_map(|address| Url::parse(address).ok())
    {
        crate::open_externally(app, &url);
    }

    if let Some(channels) = state.muted {
        inbox::replace_mutes(app, entry_id, &channels);
    }

    let Some(rail) = state.rail else {
        return;
    };

    // Membership before order (a move changes which list a server is ordered in, and forces the
    // order to be rewritten to break position ties); pruning last, so a dissolving folder's slot
    // passes to the server it frees.
    let store = app.state::<Store>();
    let changed = apply_creates(&store, &rail.creates) | apply_moves(&store, &rail.moves);

    apply_order(&store, &rail.order, changed);

    if changed {
        let _ = store.update(|registry| Ok(registry.rail().prune_folders()));
    }
}

#[derive(Debug, Deserialize)]
struct RailState {
    /// the top level (folders and loose servers) in tile order
    order: Vec<RailRef>,
    #[serde(default)]
    moves: Vec<RailMove>,
    #[serde(default)]
    creates: Vec<RailCreate>,
}

/// A server moved into (`folder_id`) or out of (`None`) a folder.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RailMove {
    server_id: String,
    folder_id: Option<String>,
}

/// A folder made by dropping one server onto another. The id is the page's (so it can draw the
/// folder at once); an id already in use is refused.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RailCreate {
    id: String,
    name: String,
    member_ids: Vec<String>,
}

fn log_failure(what: &str, result: Result<()>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[shiver] could not {what}: {error}");

            false
        }
    }
}

/// Makes the folders the rail asked for (a bounded number per poll), at the lowest position of
/// their members. Names are clamped; renaming stays on Shiver's own screen.
fn apply_creates(store: &Store, creates: &[RailCreate]) -> bool {
    if creates.is_empty() {
        return false;
    }

    let result = store.update(|registry| {
        for create in creates.iter().take(MAX_CREATES_PER_POLL) {
            let id = create.id.trim();

            if id.is_empty()
                || id.len() > MAX_FOLDER_ID
                || registry.folders.iter().any(|folder| folder.id == id)
            {
                continue;
            }

            let members: Vec<String> = create
                .member_ids
                .iter()
                .filter(|member| registry.server(member).is_some())
                .cloned()
                .collect();

            // a folder of nothing would only be pruned again
            if members.is_empty() {
                continue;
            }

            let name = match create.name.trim() {
                "" => DEFAULT_FOLDER_NAME,
                name => name,
            };

            registry
                .rail()
                .create_folder(id.to_string(), name, &members)?;
        }

        Ok(())
    });

    log_failure("make a folder from the rail", result)
}

/// Moves servers between existing folders; nothing else about folders can be changed from a page.
fn apply_moves(store: &Store, moves: &[RailMove]) -> bool {
    if moves.is_empty() {
        return false;
    }

    let result = store.update(|registry| {
        let mut rail = registry.rail();

        for change in moves {
            // a stale page may name what is gone; the rest still applies
            let _ = rail.set_server_folder(&change.server_id, change.folder_id.clone());
        }

        Ok(())
    });

    log_failure("move a server between folders", result)
}

/// Stores the rail's top-level order when it differs from the stored one (or `force`). Folder
/// contents keep their own order. Compared first because the rail reports every second.
fn apply_order(store: &Store, ordered: &[RailRef], force: bool) {
    if !force && top_level_matches(&store.registry(), ordered) {
        return;
    }

    let result = store.update(|registry| {
        let mut rail = registry.rail();

        for (index, item) in ordered.iter().enumerate() {
            let _ = rail.place(item, index as i32);
        }

        registry.servers.sort_by_key(|server| server.position);

        Ok(())
    });

    log_failure("store the rail's new order", result);
}

fn top_level_matches(registry: &crate::model::Registry, ordered: &[RailRef]) -> bool {
    let mut current: Vec<(&str, &str, i32)> = registry
        .folders
        .iter()
        .map(|folder| ("folder", folder.id.as_str(), folder.position))
        .chain(
            registry
                .servers
                .iter()
                .filter(|server| server.folder_id.is_none())
                .map(|server| ("server", server.id.as_str(), server.position)),
        )
        .collect();

    current.sort_by_key(|(_, _, position)| *position);

    current.len() == ordered.len()
        && current
            .iter()
            .zip(ordered)
            .all(|((kind, id, _), item)| *kind == item.kind && *id == item.id)
}

/// Whether the webview may navigate to `target`; records home on the very first navigation (which
/// is Shiver's own page loading, before anything else could).
pub fn is_allowed(app: &AppHandle, target: &Url, server_origin: Option<&str>) -> bool {
    let showing = app.state::<Showing>();
    let home = showing.home();
    let allowed = navigation_allowed(home.as_deref(), server_origin, target);

    if home.is_none() && server_origin.is_none() {
        showing.set_home_if_unset(target.to_string());
    }

    allowed
}

/// The open server's origin and Shiver's own pages are allowed; anything else belongs in the
/// browser. Before home is known, only Shiver's own page can be loading.
pub fn navigation_allowed(home: Option<&str>, server_origin: Option<&str>, target: &Url) -> bool {
    if server_origin.is_some_and(|origin| is_same_origin(origin, target)) {
        return true;
    }

    home.map_or(true, |home| same_place(home, target))
}

/// Same scheme, host and port as Shiver's home.
fn same_place(home: &str, target: &Url) -> bool {
    Url::parse(home).is_ok_and(|home| {
        home.scheme() == target.scheme()
            && home.host_str() == target.host_str()
            && home.port_or_known_default() == target.port_or_known_default()
    })
}

/// Called on every page load of Shiver's own pages: the bridge's "back" is a plain navigation that
/// runs no command, so this is where the webview stops showing a server (and `sync` gives that
/// server a background socket again).
pub fn landed_home(app: &AppHandle) {
    let showing = app.state::<Showing>();

    if showing.server().is_none() {
        return;
    }

    showing.set_server(None);
    inbox::sync(app);
}

pub fn is_home(app: &AppHandle, target: &Url) -> bool {
    app.state::<Showing>()
        .home()
        .is_some_and(|home| same_place(&home, target))
}

/// The webview's own background (shown between documents, white by default on Android), taken
/// from the user's theme colour.
pub fn background_color(settings: &Settings) -> Color {
    let [r, g, b] = shiver_core::model::rgb(&settings.theme_color).unwrap_or([0x0a; 3]);

    Color(r, g, b, 0xff)
}

fn theme_payload(settings: &Settings) -> Value {
    shiver_core::model::theme_payload(
        &settings.theme_color,
        &settings.accent_color,
        settings.text_color.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("a url")
    }

    #[test]
    fn the_document_start_script_seeds_and_themes_without_reaching_shiver() {
        // a clean checkout compiles against a placeholder until `bun run build:bridge` has run
        if DOCUMENT_START.starts_with("/* not built yet") {
            return;
        }

        let script = document_start();

        assert!(script.contains(seed_key()) && !DOCUMENT_START.contains(seed_key()));

        for needed in [
            SEED_PARAM,
            "SHIVER_SEED_KEY",
            "vite-ui-theme",
            "prefers-color-scheme",
            "MutationObserver",
            "__SHIVER_SESSION_SHIM__",
        ] {
            assert!(
                DOCUMENT_START.contains(needed),
                "the script should mention {needed}"
            );
        }

        for forbidden in ["__TAURI", "invoke", "fetch(", "XMLHttpRequest", "eval("] {
            assert!(
                !DOCUMENT_START.contains(forbidden),
                "the script must not contain {forbidden}"
            );
        }
    }

    #[test]
    fn seed_keys_are_per_origin() {
        let key = |value| seed_key_for("secret", &url(value));

        assert_eq!(
            key("https://chat.example.com/a"),
            "98aee65411ec7dc40877b6a4f3f4215e6b200ca75c91c8f218a22d4c7f8a43ff"
        );
        assert_eq!(
            key("https://chat.example.com:443"),
            key("https://chat.example.com")
        );
        assert_ne!(
            key("https://chat.example.com"),
            key("https://other.example.com")
        );
    }

    #[test]
    fn a_seed_never_leaves_in_a_url() {
        let seeded = url("https://sso.example.com/login#shiver-seed=secret");

        assert_eq!(
            without_seed(&seeded).as_str(),
            "https://sso.example.com/login"
        );
        assert_eq!(
            without_seed(&url("https://example.com/a#part")).as_str(),
            "https://example.com/a#part"
        );
    }

    #[test]
    fn only_a_server_still_loading_is_loading() {
        let showing = Showing::default();

        assert!(!showing.loading());
        showing.set_server(Some("a".into()));
        assert!(showing.loading());
        showing.set_loaded();
        assert!(!showing.loading());
        showing.set_server(Some("b".into()));
        assert!(showing.loading());
    }

    #[test]
    fn an_unreadable_setting_still_paints_dark() {
        let settings = Settings {
            theme_color: "rgb(255, 0, 0)".into(),
            ..Default::default()
        };

        assert_eq!(background_color(&settings), Color(0x0a, 0x0a, 0x0a, 0xff));
    }

    #[test]
    fn navigation_is_pinned_to_home_and_the_open_server() {
        let home = Some("http://localhost:1421/");
        let server = Some("https://chat.example.com");

        // before home is known, the first navigation is Shiver's own
        assert!(navigation_allowed(
            None,
            None,
            &url("http://localhost:1421/")
        ));
        assert!(navigation_allowed(
            home,
            None,
            &url("http://localhost:1421/index.html")
        ));
        assert!(navigation_allowed(
            home,
            server,
            &url("https://chat.example.com/channels/1")
        ));
        assert!(navigation_allowed(
            home,
            server,
            &url("http://localhost:1421/")
        ));

        assert!(!navigation_allowed(
            home,
            server,
            &url("https://elsewhere.example.com/")
        ));
        assert!(!navigation_allowed(
            Some("http://tauri.localhost/"),
            None,
            &url("http://localhost:1421/")
        ));
        assert!(!navigation_allowed(
            home,
            None,
            &url("http://localhost:1420/")
        ));
    }

    fn entry(id: &str, name: &str, origin: &str, position: i32) -> ServerEntry {
        ServerEntry {
            id: id.into(),
            origin: origin.into(),
            name: name.into(),
            icon_url: Some(format!("{origin}/public/logo.png")),
            identity: Some("someone".into()),
            account_label: Some("someone".into()),
            push_token: Some("secret-push-token".into()),
            position,
            ..Default::default()
        }
    }

    #[test]
    fn the_rail_payload_carries_no_addresses_and_is_ordered() {
        let servers = vec![
            entry("a", "Alpha", "https://alpha.example.com", 7),
            entry("b", "Beta", "https://beta.example.com", 2),
        ];

        let payload = rail_payload(&servers, &HashMap::new(), &["a".into()]);
        let text = payload.to_string();

        assert!(text.contains("Alpha") && text.contains("Beta"));

        for hidden in ["example.com", "someone", "secret-push-token"] {
            assert!(!text.contains(hidden), "the rail must not carry {hidden}");
        }

        assert_eq!(payload[0]["id"], "b");
        assert_eq!(payload[1]["id"], "a");
        assert_eq!(payload[1]["signedOut"], true);
    }

    #[test]
    fn pending_requests_are_consumed_once_and_home_is_written_once() {
        let showing = Showing::default();

        showing.set_pending_dm_user(Some("ana".into()));
        assert_eq!(showing.take_pending_dm_user().as_deref(), Some("ana"));
        assert_eq!(showing.take_pending_dm_user(), None);

        showing.set_home_if_unset("http://localhost:1421/".into());
        showing.set_home_if_unset("http://tauri.localhost/".into());
        assert_eq!(showing.home().as_deref(), Some("http://localhost:1421/"));
    }

    #[test]
    fn a_page_state_tolerates_missing_and_null_fields() {
        let state: PageState =
            serde_json::from_str(r#"{"muted":null,"rail":null,"open":null}"#).unwrap();

        assert!(state.muted.is_none() && state.rail.is_none() && state.open.is_empty());

        let state: PageState =
            serde_json::from_str(r#"{"rail":{"order":[{"kind":"server","id":"a"}]}}"#).unwrap();

        assert_eq!(state.rail.unwrap().order.len(), 1);
    }
}
