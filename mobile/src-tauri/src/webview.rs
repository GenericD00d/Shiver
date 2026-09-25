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

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use shiver_core::LockExt;
use tauri::{window::Color, AppHandle, Emitter, Manager, Url, WebviewWindow};

pub use shiver_core::limit::Openings;

use crate::{
    error::{Core, Error, Result},
    inbox,
    model::{is_same_origin, ServerEntry, Settings, MAX_SOUND_VOLUME},
    store::Store,
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

const MAX_PAGE_STATE_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct ShowingState {
    /// where Shiver's own pages are; first write wins (dev server and app URL differ)
    home: Option<String>,
    /// the entry on screen, if any
    server: Option<String>,
    loaded: bool,
    /// a page Shiver did not open is loading (a step through history), on its way home
    stray: bool,
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
        state.stray = false;
    }

    pub fn set_stray(&self) {
        self.0.locked().stray = true;
    }

    /// Whether Shiver's own page is what the webview shows (or is loading).
    pub fn at_home(&self) -> bool {
        let state = self.0.locked();

        state.server.is_none() && !state.stray
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

/// Tells Shiver's own page something, but only while it is on screen: the one webview keeps the
/// page's listeners across navigations, so an event sent while a server's page is up would be
/// evaluated in that page. Shiver's page re-reads everything when it next loads.
pub fn emit_home(app: &AppHandle, event: &str, payload: impl Serialize + Clone) {
    if app.state::<Showing>().at_home() {
        let _ = app.emit(event, payload);
    }
}

pub fn main_window(app: &AppHandle) -> Result<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW)
        .ok_or_else(|| Error::Webview("The Shiver window is not open".into()))
}

/// The still Android took of `origin`'s page as the user left it (`MainActivity.takeStill`), as
/// a `data:` uri. Taken once; a still of another page is dropped.
#[cfg(target_os = "android")]
pub async fn take_still(app: &AppHandle, origin: String) -> Option<String> {
    use jni::objects::{JString, JValue};

    let (sender, receiver) = tokio::sync::oneshot::channel();

    main_window(app)
        .ok()?
        .with_webview(move |platform| {
            platform.jni_handle().exec(move |env, activity, _| {
                let take = || -> jni::errors::Result<Option<String>> {
                    let origin = env.new_string(origin)?;
                    let still = env
                        .call_method(
                            activity,
                            "takeStill",
                            "(Ljava/lang/String;)Ljava/lang/String;",
                            &[JValue::Object(&origin)],
                        )?
                        .l()?;

                    if still.is_null() {
                        return Ok(None);
                    }

                    Ok(Some(env.get_string(&JString::from(still))?.into()))
                };
                let still = take();

                if still.is_err() {
                    let _ = env.exception_clear();
                }

                let _ = sender.send(still.ok().flatten());
            });
        })
        .ok()?;

    tokio::time::timeout(std::time::Duration::from_secs(2), receiver)
        .await
        .ok()?
        .ok()?
}

/// Only Android takes stills.
#[cfg(not(target_os = "android"))]
pub async fn take_still(_app: &AppHandle, _origin: String) -> Option<String> {
    None
}

/// Drops the still Android may still hold, as a server is opened (no origin matches none).
fn forget_still(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        take_still(&app, String::new()).await;
    });
}

/// Navigates to a server's client, seeding `token` through the URL fragment when there is one.
pub fn show_server(app: &AppHandle, entry: &ServerEntry, token: Option<&str>) -> Result<()> {
    let window = main_window(app)?;

    forget_still(app);

    let mut url = Url::parse(&entry.origin)
        .map_err(|_| Core::InvalidOrigin(format!("'{}' is not a valid address", entry.origin)))?;

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
    if let Some(id) = app.state::<Showing>().server() {
        go_home(app, Some(&format!("failed={id}")));
    }
}

/// Sends the webview to Shiver's own page, with a fragment for it to act on.
pub fn go_home(app: &AppHandle, fragment: Option<&str>) {
    let Some(Ok(mut url)) = app.state::<Showing>().home().map(|home| Url::parse(&home)) else {
        return;
    };

    url.set_fragment(fragment);

    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        if let Ok(window) = main_window(&app) {
            let _ = window.navigate(url);
        }
    });
}

/// Everything one server page is handed when the bridge is installed in it: its own entry's, and
/// nothing about the user's other servers.
pub struct PageContext<'a> {
    pub entry: &'a ServerEntry,
    pub settings: &'a Settings,
    /// channel ids muted on this entry
    pub muted: &'a [i64],
    /// this entry's own session, never another's (used by the bridge to reconnect)
    pub session: Option<&'a str>,
    /// the unread floor to share with the user's other devices through the companion plugin
    pub read_floor: Option<&'a HashMap<i64, u32>>,
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
        "pushEndpoint": page.push_endpoint,
        "retiredPushEndpoints": page.retired_push_endpoints,
        "openDmUser": showing.take_pending_dm_user(),
        "session": page.session,
    });

    let _ = window.eval(format!("window.__SHIVER__ = {config};\n{BRIDGE_SOURCE}"));
}

/// Polled by `inbox::watch_mutes`: reads the page's mutes and queued outside links (a read on
/// departure would be torn down by the navigation before it answered).
const READ_SCRIPT: &str = "JSON.stringify({ \
    muted: window.__SHIVER_MUTED__ ? window.__SHIVER_MUTED__() : null, \
    open: window.__SHIVER_OPEN__ ? window.__SHIVER_OPEN__() : null })";

#[derive(Default, Deserialize)]
struct PageState {
    muted: Option<Vec<i64>>,
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
    // what the page says the user opened away from it; opened only if the user agrees
    let server = app
        .state::<Store>()
        .registry()
        .server(entry_id)
        .map(|server| server.name.clone());

    for url in app
        .state::<Openings>()
        .grant(entry_id, &state.open)
        .iter()
        .filter_map(|address| Url::parse(address).ok())
    {
        crate::ask_to_open(app, server.as_deref().unwrap_or("A server"), &url);
    }

    if let Some(channels) = state.muted {
        inbox::replace_mutes(app, entry_id, &channels);
    }
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

/// Called when Shiver's own page starts loading in the main frame: going back from a server is a
/// plain navigation that runs no command, so this is where the webview stops showing a server (and
/// `sync` gives that server a background socket again).
pub fn landed_home(app: &AppHandle) {
    let showing = app.state::<Showing>();

    if showing.at_home() {
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
    fn home_is_only_home_while_nothing_else_is_on_its_way() {
        let showing = Showing::default();

        assert!(showing.at_home());
        showing.set_stray();
        assert!(!showing.at_home());
        showing.set_server(None);
        assert!(showing.at_home());
        showing.set_server(Some("a".into()));
        assert!(!showing.at_home());
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
        let state: PageState = serde_json::from_str(r#"{"muted":null,"open":null}"#).unwrap();

        assert!(state.muted.is_none() && state.open.is_empty());

        let state: PageState = serde_json::from_str(r#"{"muted":[3]}"#).unwrap();

        assert_eq!(state.muted, Some(vec![3]));
    }
}
