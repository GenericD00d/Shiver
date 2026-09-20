//! The one webview, and what it is showing.
//!
//! Android gives a window a single webview — `add_child` is `#[cfg(desktop)]` in Tauri itself — so
//! Shiver here is a switcher rather than a shell: the webview shows either Shiver's own pages or one
//! server's client, and opening a server is a navigation.
//!
//! That removes the desktop client's central problem and replaces it with a smaller one.
//!
//! **Mobile does hold credentials**, contrary to what this file used to say. `keyring` has no
//! Android backend, so Shiver brings its own: `tauri-plugin-shiver-secrets` keeps the session — and
//! the password, when the user asks it to — in `EncryptedSharedPreferences`, with the master key in
//! the Android Keystore. `install_bridge` below seeds that session into the page exactly as desktop
//! does.
//!
//! What is different is *when*. Desktop injects the bridge as an initialization script because it
//! has to beat the page's own scripts to `localStorage`. Here the bridge is evaluated after the
//! page loads, which means the page's scripts run first and define the environment the bridge then
//! finds — so nothing the bridge reads back out of a page (`read_mutes`) may be treated as more
//! than a request. One webview, no recreation, no initialization script.

use std::sync::Mutex;

use serde_json::json;
use tauri::{AppHandle, Manager, Url, WebviewWindow};

use tauri_plugin_shiver_secrets::SecretsExt;

use crate::store::RegistryStore;
use crate::{
    error::{Error, Result},
    model::{is_same_origin, ServerEntry, Settings},
};

pub const MAIN_WINDOW: &str = "main";

const BRIDGE_SOURCE: &str = include_str!("../generated/bridge.js");

/// How many links one page may have opened in the browser recently.
///
/// The bridge leaves addresses for the core to open, because Android's webview refuses to make the
/// new window Sharkord asks for. Those addresses come from the page, and `read_mutes` polls for
/// them on a timer — so a page that queues hundreds turns this into a stream of browser tabs, with
/// no user gesture anywhere in it.
///
/// Desktop already ships exactly this guard (`drain.rs`, `Openings`). Mobile polls the same way and
/// had no limit at all, which is the whole of the difference.
#[derive(Default)]
pub struct Openings(Mutex<Vec<std::time::Instant>>);

/// The most links one poll may open. A tap opens one, so this is generous.
const OPEN_PER_TICK: usize = 5;

/// And the most within `OPEN_WINDOW`, so a page cannot simply queue again at the next poll.
const OPEN_PER_WINDOW: usize = 10;
const OPEN_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

impl Openings {
    /// How many more may be opened now, forgetting whatever has aged out of the window.
    fn allowance(&self) -> usize {
        let mut recent = self.0.lock().unwrap_or_else(|e| e.into_inner());

        recent.retain(|at| at.elapsed() < OPEN_WINDOW);

        OPEN_PER_WINDOW
            .saturating_sub(recent.len())
            .min(OPEN_PER_TICK)
    }

    fn record(&self) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(std::time::Instant::now());
    }
}

/// Where Shiver's own pages live, and which server the webview is on.
///
/// The home URL is captured at startup rather than constructed: it differs between a dev server and
/// a bundled app, and the bridge needs the exact one to send the user back to.
#[derive(Default)]
pub struct Showing {
    home: Mutex<Option<String>>,
    server: Mutex<Option<String>>,
    /// set when the user reached this server by tapping the rail's direct-messages tile, and read
    /// once by the bridge so it can open Sharkord's own dm list on arrival
    pending_dms: Mutex<bool>,
    /// the conversation to open on arrival, named by the person it is with
    ///
    /// A name rather than the channel id, because the rows in Sharkord's own list carry neither an
    /// id nor anything else to match on — the name is what is actually on screen to be found.
    pending_dm_user: Mutex<Option<String>>,
}

impl Showing {
    /// Records where Shiver's own pages are, the first time anything knows.
    ///
    /// First write wins. There are two callers — the webview's opening navigation, and the URL the
    /// window reports once built — and they do not agree in development: the navigation is the vite
    /// dev server, while the window reports its own app URL. Letting the second overwrite the first
    /// made every later navigation to the dev server look foreign, so Shiver handed its own page to
    /// the system browser and opened a localhost tab on every reload.
    pub fn set_home_if_unset(&self, url: String) {
        let mut home = self.home.lock().unwrap_or_else(|e| e.into_inner());

        if home.is_none() {
            *home = Some(url);
        }
    }

    pub fn home(&self) -> Option<String> {
        self.home.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_server(&self, entry_id: Option<String>) {
        *self.server.lock().unwrap_or_else(|e| e.into_inner()) = entry_id;
    }

    pub fn server(&self) -> Option<String> {
        self.server
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_pending_dms(&self, pending: bool) {
        *self.pending_dms.lock().unwrap_or_else(|e| e.into_inner()) = pending;
    }

    /// Reads the flag and clears it, so opening the dm list happens on arrival and not again on
    /// every reload or in-server navigation after it.
    pub fn set_pending_dm_user(&self, user: Option<String>) {
        *self
            .pending_dm_user
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = user;
    }

    /// Reads the pending conversation and clears it, so a later reload does not reopen it.
    pub fn take_pending_dm_user(&self) -> Option<String> {
        self.pending_dm_user
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub fn take_pending_dms(&self) -> bool {
        let mut pending = self.pending_dms.lock().unwrap_or_else(|e| e.into_inner());

        std::mem::replace(&mut pending, false)
    }
}

pub fn main_window(app: &AppHandle) -> Result<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW)
        .ok_or_else(|| Error::Webview("The Shiver window is not open".into()))
}

/// Takes Shiver's seeded sign-in back out of the page that is about to be left.
///
/// The session Shiver holds lives encrypted, in the Android Keystore's care. Handing it to a server's
/// client necessarily writes it into that page's `localStorage`, which is an ordinary file on disk —
/// so leaving it there between visits would undo the encrypted store for anyone able to read the
/// app's data. It goes in when the server is opened and comes out when it is left.
///
/// Issued before the navigation that replaces the page, and both travel the same queue to the
/// webview, so the clean-up runs first. The bridge ignores it unless Shiver was the one that seeded
/// that page, so a sign-in the user asked the server to remember is left alone.
fn forget_page_session(app: &AppHandle) {
    let Ok(window) = main_window(app) else {
        return;
    };

    let _ = window.eval("window.__SHIVER_FORGET_SESSION__ && window.__SHIVER_FORGET_SESSION__()");
}

/// What Shiver carries across a wipe, per entry, in the encrypted store.
///
/// Everything the *user* chose, as opposed to everything the *session* is. Leaving a server drops
/// that origin's storage, and the storage is where Sharkord keeps its settings — so carrying only
/// the channel and the drafts meant every other preference was destroyed on every server switch.
/// The notification toggles were the visible case: they read as off on every visit, whatever the
/// user had saved, because the keys behind them had been wiped minutes earlier.
///
/// Encrypted, because a half-written message is the user's and has no business sitting in a plain
/// file either.
fn carried_key(entry_id: &str) -> String {
    crate::inbox::carried_store_key(entry_id)
}

/// Reads the two values worth keeping, then drops everything that server's page put on the disk.
///
/// The order matters and so does the failure mode. The read happens first and the wipe only follows
/// it, so a read that fails leaves the storage alone: the session has already been cleared by
/// `forget_page_session`, and losing someone's unsent message to make a wipe tidy is not a trade
/// Shiver gets to make.
///
/// Skipped when the next page is the same origin — two entries can share one server, and wiping
/// then would take the storage out from under the page just being opened.
fn carry_and_wipe(app: &AppHandle, leaving: &ServerEntry, next_origin: Option<&str>) {
    if next_origin == Some(leaving.origin.as_str()) {
        return;
    }

    let Ok(window) = main_window(app) else {
        return;
    };

    let handle = app.clone();
    let entry_id = leaving.id.clone();
    let origin = leaving.origin.clone();

    // A bare expression: an immediately-invoked function with a `try` in it comes back from
    // `eval_with_callback` as nothing at all, which is how this failed the first time it was tried.
    //
    // Everything the client keeps under its own prefix — and Shiver's own, which is where the bridge
    // records that this device has already handed its mutes to the plugin; losing that on a server
    // switch would have it adopt them again. Minus the three keys that *are* the
    // session — Shiver holds that, and putting it back would undo `forget_page_session`. Capped, so a
    // page cannot use the encrypted store as a dumping ground by writing a megabyte under a
    // `sharkord-` key: over the cap nothing is carried, which costs settings rather than silently
    // keeping half of them.
    let script = concat!(
        "JSON.stringify((function(){var out={},total=0;",
        "var skip=['sharkord-identity','sharkord-auto-login','sharkord-auto-login-token'];",
        "for(var i=0;i<localStorage.length;i++){var k=localStorage.key(i);",
        "if(!k||skip.indexOf(k)>=0)continue;",
        "if(k.indexOf('sharkord-')!==0&&k.indexOf('shiver-')!==0&&k!=='vite-ui-theme')continue;",
        "var v=localStorage.getItem(k);if(v===null)continue;",
        "total+=k.length+v.length;if(total>65536)return null;out[k]=v}",
        "return out})())"
    );

    let _ = window.eval_with_callback(script, move |raw| {
        // off this thread before touching the plugin: a blocking call into the JVM from inside an
        // eval callback deadlocks the app, which is the ANR this cost once already
        let inner = handle.clone();
        let id = entry_id.clone();
        let origin = origin.clone();

        std::thread::spawn(move || {
            let Ok(serde_json::Value::String(carried)) =
                serde_json::from_str::<serde_json::Value>(&raw)
            else {
                // nothing came back, so nothing is known to be safe to destroy
                return;
            };

            if let Err(error) = inner.shiver_secrets().set(&carried_key(&id), &carried) {
                eprintln!("[shiver] could not keep {id}'s drafts across the wipe: {error}");

                return;
            }

            // mirrored so the next page load can read it without waiting on the plugin
            inner
                .state::<crate::inbox::Inbox>()
                .remember_carried(&id, &carried);

            if let Err(error) = inner.shiver_secrets().wipe_origin(&origin) {
                eprintln!("[shiver] could not clear {origin} from the webview: {error}");
            }
        });
    });
}

/// The values kept from this entry's last visit, ready to be put back into its page.
///
/// Read from memory, never from the store. This runs on the page-load hook, which is the Android
/// main thread, and `run_mobile_plugin` blocks that thread waiting on a reply the same thread has to
/// deliver — so asking the plugin here wedges the app the moment a server's page finishes loading.
/// The mirror is filled by `carry_and_wipe` and by `inbox::restore`, both of which are off it.
pub fn carried_state(app: &AppHandle, entry_id: &str) -> Option<String> {
    app.state::<crate::inbox::Inbox>().carried(entry_id)
}

/// Points the webview at a server's client.
pub fn show_server(app: &AppHandle, entry: &ServerEntry) -> Result<()> {
    let window = main_window(app)?;

    forget_page_session(app);
    leaving(app, Some(&entry.origin));

    let url = Url::parse(&entry.origin)
        .map_err(|_| Error::InvalidOrigin(format!("'{}' is not a valid address", entry.origin)))?;

    app.state::<Showing>().set_server(Some(entry.id.clone()));

    window
        .navigate(url)
        .map_err(|error| Error::Webview(error.to_string()))
}

/// Points the webview back at Shiver's own pages.
pub fn show_shiver(app: &AppHandle) -> Result<()> {
    let window = main_window(app)?;

    forget_page_session(app);
    leaving(app, None);

    let Some(home) = app.state::<Showing>().home() else {
        return Err(Error::Webview(
            "Shiver does not know where its own pages are".into(),
        ));
    };

    let url = Url::parse(&home)
        .map_err(|_| Error::Webview("Shiver's own address is unreadable".into()))?;

    app.state::<Showing>().set_server(None);

    window
        .navigate(url)
        .map_err(|error| Error::Webview(error.to_string()))
}

/// Runs the bridge in a server page that has just finished loading.
///
/// Evaluated rather than injected: see the module note. It carries only this entry's own data, the
/// same rule as on desktop — nothing about any other server is reachable from a server's page.
/// Everything one page is handed when the bridge is installed in it.
///
/// A struct rather than a long argument list, because the list had grown to the point where the
/// call site said nothing about which value was which — and two of these are a session and another
/// server's data, which are exactly the arguments not to get the wrong way round.
pub struct PageContext<'a> {
    pub entry: &'a ServerEntry,
    pub settings: &'a Settings,
    /// channel ids this user muted on this entry
    pub muted: &'a [i64],
    /// every server, for the rail — reduced by `rail_payload` before it reaches the page
    pub servers: &'a [ServerEntry],
    pub unread: &'a std::collections::HashMap<String, u32>,
    /// servers whose session expired and that are waiting for the user to sign in
    pub signed_out: &'a [String],
    /// this server's own session, never another's
    pub session: Option<&'a str>,
    /// what was kept when this server's storage was last wiped
    pub carried: Option<&'a str>,
    /// The unread floor to store in the companion plugin, so this user's devices share one badge.
    ///
    /// Arriving at a server is the one thing that moves the floor, and this page *is* that arrival
    /// — so the value travels with it rather than being evaluated in afterwards. Empty where there
    /// is nothing to say, which is any server Shiver has not yet connected to.
    pub read_floor: Option<&'a std::collections::HashMap<i64, u32>>,
    /// the rail's folders, so the tiles inside a server's page group the way Shiver's own do
    pub folders: &'a [crate::model::Folder],
    /// **this entry's** push endpoint, so the page can hand it to the Shiver plugin's relay. Never
    /// another entry's: it is a capability to wake this phone about this server, and the same rule
    /// that keeps addresses out of a page keeps everyone else's endpoint out of it too.
    pub push_endpoint: Option<&'a str>,
}

pub fn install_bridge(app: &AppHandle, page: PageContext<'_>) {
    let Ok(window) = main_window(app) else {
        return;
    };

    let Some(home) = app.state::<Showing>().home() else {
        return;
    };

    let config = json!({
        "entryId": page.entry.id,
        "origin": page.entry.origin,
        "serverName": page.entry.name,
        "theme": theme_payload(page.settings),
        "muted": page.muted,
        "soundVolume": page.settings.sound_volume.min(crate::model::MAX_SOUND_VOLUME),
        "minimiseAttachments": page.settings.minimise_attachments,
        // json object keys are strings, so the channel ids go over as strings and are parsed back
        // on the way in — see `parse_shared_floor` in `sharkord-client`
        "readFloor": page.read_floor.map(|floor| {
            floor
                .iter()
                .map(|(channel_id, count)| (channel_id.to_string(), *count))
                .collect::<std::collections::HashMap<String, u32>>()
        }),
        // where the back control sends the user. the page cannot call Shiver — it has no IPC, by the
        // same rule that protects the desktop client — so going back is a navigation, not a call.
        "home": home,
        "rail": rail_payload(page.servers, page.unread, page.signed_out),
        "folders": folder_payload(page.folders),
        // No conversation list. The page draws its own from Sharkord's store — which is its own
        // data, already in reach — and everything cross-server lives on Shiver's own screen, which is
        // the only place it can be shown without handing one server the names of the people the
        // user talks to on all the others.
        // Given so the bridge can register it with the companion plugin, which checks it and stores
        // it against this user. Absent when there is no distributor, or none has answered yet.
        "pushEndpoint": page.push_endpoint,
        "openDms": app.state::<Showing>().take_pending_dms(),
        "openDmUser": app.state::<Showing>().take_pending_dm_user(),
        // This server's own session, and only ever this one. It is the token that belongs to the
        // page being handed it, so nothing crosses that was not already the page's to know — the
        // rail's data stays free of every other server's, which is the rule `rail_payload` keeps.
        "session": page.session,
        // what survived the wipe of this server's storage: the channel it was left on and any
        // unsent drafts, both of which are this page's own to begin with
        "carried": page.carried,
    });

    let _ = window.eval(format!("window.__SHIVER__ = {config};\n{BRIDGE_SOURCE}"));
}

/// The rail, as much of it as a server's page is allowed to know.
///
/// On desktop the rail is Shiver's own webview and a server's page is handed nothing about any other
/// server. Android has one webview per window, so the rail has to be drawn inside the page the user
/// is looking at, and it cannot be drawn from nothing.
///
/// What crosses is the least that still draws a rail: a display name, the logo as bytes, and the
/// opaque entry id. What deliberately does not cross is every address — `origin`, `iconUrl`, the
/// account label and the identity. So a hostile server learns what the user's other servers are
/// called and not one way to reach them, and tapping a tile navigates to Shiver's own page carrying
/// only that id, which Shiver resolves to an address on its side.
fn rail_payload(
    servers: &[ServerEntry],
    unread: &std::collections::HashMap<String, u32>,
    signed_out: &[String],
) -> serde_json::Value {
    let mut ordered: Vec<&ServerEntry> = servers.iter().collect();

    ordered.sort_by_key(|server| server.position);

    serde_json::Value::Array(
        ordered
            .into_iter()
            .map(|server| {
                json!({
                    "id": server.id,
                    "name": server.name,
                    "icon": server.icon_data,
                    // where it sits, and which folder it sits in. Both are the rail's own shape
                    // rather than anything about the server, and the rail cannot be drawn without
                    // them once folders exist.
                    "position": server.position,
                    "folderId": server.folder_id,
                    // a count, which is not an address: the rail may say a neighbour has messages
                    // waiting without saying where that neighbour is
                    "unread": unread.get(&server.id).copied().unwrap_or(0),
                    // Shiver's session for this one expired and it has no way to renew it, so the
                    // tile says so rather than the server just going quiet
                    "signedOut": signed_out.iter().any(|id| id == &server.id),
                })
            })
            .collect(),
    )
}

/// The folders, which are Shiver's own furniture rather than anything a server owns.
///
/// A name and a position, the same shape of thing the server list is already allowed to carry. The
/// name is one the user typed into Shiver, so it says nothing about any server — but it does cross
/// into a server's page, which is why it is listed here rather than assumed harmless.
fn folder_payload(folders: &[crate::model::Folder]) -> serde_json::Value {
    let mut ordered: Vec<&crate::model::Folder> = folders.iter().collect();

    ordered.sort_by_key(|folder| folder.position);

    serde_json::Value::Array(
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
            .collect(),
    )
}

/// Reads a server's muted channels out of its own page.
///
/// Called on a timer by `inbox::watch_mutes` rather than once on the way out, and the difference is
/// load-bearing. `eval_with_callback` answers asynchronously, and a departure issues `navigate`
/// immediately after — which tears the page down before the answer comes back, so a read done there
/// silently never lands. The value has to already be in hand by the time the user leaves.
pub fn read_mutes(app: &AppHandle, entry_id: &str) {
    let Ok(window) = main_window(app) else {
        return;
    };

    let handle = app.clone();
    let entry_id = entry_id.to_string();

    // One read for all of it, because it is all things the rail changed and the page is the only
    // place any of it is known. A bare expression producing a string: an immediately-invoked
    // function with a `try` in it comes back from `eval_with_callback` as nothing at all, which is
    // how the token harvest failed once.
    let script = "JSON.stringify({ \
                  muted: window.__SHIVER_MUTED__ ? window.__SHIVER_MUTED__() : null, \
                  rail: window.__SHIVER_RAIL_STATE__ ? window.__SHIVER_RAIL_STATE__() : null, \
                  open: window.__SHIVER_OPEN__ ? window.__SHIVER_OPEN__() : null })";

    let _ = window.eval_with_callback(script, move |raw| {
        // arrives json-encoded, so the object is a string that still has to be parsed
        let Ok(serde_json::Value::String(body)) = serde_json::from_str::<serde_json::Value>(&raw)
        else {
            // no bridge on this page
            return;
        };

        let Ok(state) = serde_json::from_str::<serde_json::Value>(&body) else {
            return;
        };

        let channels = serde_json::from_value::<Vec<i64>>(state["muted"].clone()).ok();
        let rail = serde_json::from_value::<RailState>(state["rail"].clone()).ok();
        let open = serde_json::from_value::<Vec<String>>(state["open"].clone()).unwrap_or_default();

        // off this thread before touching the store: the recount that follows ends in an `eval`,
        // and issuing one from inside an eval callback is the deadlock this has cost twice
        let inner = handle.clone();
        let id = entry_id.clone();

        std::thread::spawn(move || {
            // Links the page could not follow itself. Sharkord opens files and outside links in a
            // new window, which Android's webview refuses to make, so the bridge catches the click
            // and leaves the address here — the browser is where those belong anyway, and it is
            // where every other link out of Sharkord already goes.
            //
            // Rationed, because Shiver is doing this on a page's word and this is a poll rather
            // than a click. See `Openings`.
            let allowance = inner.state::<Openings>().allowance();

            if open.len() > allowance {
                // said out loud: dropping what a page asked for reads as a broken link rather than
                // as a limit being applied
                eprintln!(
                    "[shiver] {} asked to open {} addresses, opening {allowance} — the rest dropped",
                    id,
                    open.len()
                );
            }

            for address in open.into_iter().take(allowance) {
                match url::Url::parse(&address) {
                    Ok(url) => {
                        inner.state::<Openings>().record();
                        crate::open_externally(&inner, &url);
                    }
                    Err(error) => eprintln!("[shiver] the page asked to open {address}: {error}"),
                }
            }

            if let Some(channels) = channels {
                crate::inbox::replace_mutes(&inner, &id, &channels);
            }

            if let Some(rail) = rail {
                // Membership first: an order is only meaningful once each tile is in its right
                // scope, and a move changes which list a server is ordered within.
                //
                // A move also forces the order to be written. A server arriving at the top level
                // brings whatever position it held inside its folder, which can equal one already
                // in use — and the comparison below would then find the order it was asked for and
                // decline to write it, leaving two tiles tied and their order undecided.
                let made = apply_creates(&inner, &rail.creates);
                let moved = apply_moves(&inner, &rail.moves);

                apply_order(&inner, &rail.order, made || moved);

                // Last, and after the order: a folder about to dissolve is still a tile in the
                // order the page reported, so pruning any earlier would leave the server it frees
                // out of the numbering altogether. Here, the folder's place is written and then
                // handed straight over to the server that was in it.
                if made || moved {
                    let store = inner.state::<crate::store::Store>();

                    let _ = store.update(|registry| Ok(crate::model::prune_folders(registry)));
                }
            }
        });
    });
}

/// What the rail in a server's page says about itself.
#[derive(Debug, Clone, serde::Deserialize)]
struct RailState {
    /// the top level, folders and loose servers together, in the order the tiles are in
    order: Vec<RailItem>,
    /// servers the user moved into or out of a folder since the last look
    #[serde(default)]
    moves: Vec<RailMove>,
    /// folders the user made by dropping one server onto another
    #[serde(default)]
    creates: Vec<RailCreate>,
}

/// A folder the rail has already drawn and now wants Shiver to keep.
///
/// The id is the page's, not Shiver's, and that is deliberate: the rail draws the new folder the
/// instant the user makes it, and it can only do that if it knows what to call it. Shiver takes the
/// id as given rather than minting its own, so the two agree without a round trip. An id already in
/// use is refused — the page may name a new folder, not quietly redefine an existing one.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RailCreate {
    id: String,
    name: String,
    member_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RailItem {
    kind: String,
    id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RailMove {
    server_id: String,
    /// the folder it now belongs to, or nothing when it was taken out of one
    folder_id: Option<String>,
}

/// Makes the folders the user made by dragging one server onto another.
///
/// The narrowest kind of creation: a folder with a default name holding exactly the servers named,
/// and only where the id is free. Renaming stays on Shiver's own screen — a server's page may put two
/// tiles together, it may not decide what the group is called.
fn apply_creates(app: &AppHandle, creates: &[RailCreate]) -> bool {
    if creates.is_empty() {
        return false;
    }

    let store = app.state::<crate::store::Store>();

    let written = store.update(|registry| {
        for create in creates {
            if registry.folders.iter().any(|folder| folder.id == create.id) {
                continue;
            }

            // where the user dropped, rather than at the end of the rail
            let position = registry
                .servers
                .iter()
                .filter(|server| create.member_ids.contains(&server.id))
                .map(|server| server.position)
                .min()
                .unwrap_or(0);

            for (index, id) in create.member_ids.iter().enumerate() {
                if let Some(server) = registry.servers.iter_mut().find(|server| &server.id == id) {
                    server.folder_id = Some(create.id.clone());
                    server.position = index as i32;
                }
            }

            registry.folders.push(crate::model::Folder {
                id: create.id.clone(),
                name: create.name.trim().to_string(),
                position,
                expanded: true,
            });
        }

        Ok(())
    });

    if let Err(error) = written {
        eprintln!("[shiver] could not make a folder from the rail: {error}");

        return false;
    }

    true
}

/// Moves servers between folders, as asked for by the rail's own menu.
///
/// The narrowest structural change Shiver accepts from a page on a server's origin: which folder a
/// server sits in, and nothing else. Folders cannot be made, renamed or destroyed from there — a
/// server has no business rewriting the shape of the rail, only the user does, and everything else
/// is done on Shiver's own screen where the page cannot reach.
fn apply_moves(app: &AppHandle, moves: &[RailMove]) -> bool {
    if moves.is_empty() {
        return false;
    }

    let store = app.state::<crate::store::Store>();

    let written = store.update(|registry| {
        for move_ in moves {
            // a folder that has since been deleted is not a reason to refuse the rest
            // `map_or(true, ..)` rather than `is_none_or`, which is newer than this crate's
            // declared minimum rust version
            let known = move_.folder_id.as_ref().map_or(true, |id| {
                registry.folders.iter().any(|folder| &folder.id == id)
            });

            if !known {
                continue;
            }

            if let Some(server) = registry
                .servers
                .iter_mut()
                .find(|server| server.id == move_.server_id)
            {
                server.folder_id = move_.folder_id.clone();
            }
        }

        Ok(())
    });

    if let Err(error) = written {
        eprintln!("[shiver] could not move a server between folders: {error}");

        return false;
    }

    true
}

/// Persists an order the rail arrived at by dragging, when it differs from the stored one.
///
/// Only the top level: folders and the servers not in one share a position space, and the servers
/// inside a folder keep their own order. Compared before writing because this arrives on a timer —
/// the rail reports where its tiles are every second, and almost every one of those is the order
/// Shiver already has.
fn apply_order(app: &AppHandle, ordered: &[RailItem], force: bool) {
    let store = app.state::<crate::store::Store>();

    let unchanged = !force && {
        let registry = store.registry();
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
    };

    if unchanged {
        return;
    }

    let written = store.update(|registry| {
        for (index, item) in ordered.iter().enumerate() {
            let position = index as i32;

            if item.kind == "folder" {
                if let Some(folder) = registry.folders.iter_mut().find(|f| f.id == item.id) {
                    folder.position = position;
                }
            } else if let Some(server) = registry.servers.iter_mut().find(|s| s.id == item.id) {
                server.position = position;
            }
        }

        registry.servers.sort_by_key(|server| server.position);

        Ok(())
    });

    if let Err(error) = written {
        eprintln!("[shiver] could not store the rail's new order: {error}");
    }
}

/// Carries and clears the storage of whichever server is being navigated away from.
fn leaving(app: &AppHandle, next_origin: Option<&str>) {
    let Some(entry_id) = app.state::<Showing>().server() else {
        return;
    };

    let leaving = {
        let store = app.state::<crate::store::Store>();
        let registry = store.registry();

        registry
            .servers
            .iter()
            .find(|server| server.id == entry_id)
            .cloned()
    };

    if let Some(leaving) = leaving {
        carry_and_wipe(app, &leaving, next_origin);
    }
}

/// Whether a navigation is one Shiver allows the webview to make.
///
/// Two destinations are legitimate: the server the user opened, and Shiver's own pages. Everything
/// else is a link out of Sharkord and belongs in the browser, which is the same origin pinning the
/// desktop client applies — a server must not be able to move the webview somewhere Shiver still
/// labels as that server.
pub fn is_allowed(app: &AppHandle, target: &Url, server_origin: Option<&str>) -> bool {
    let showing = app.state::<Showing>();
    let home = showing.home();

    let allowed = navigation_allowed(home.as_deref(), server_origin, target);

    // The webview's very first navigation *is* Shiver's own page loading, and it happens while the
    // window is still being built, before anything has had a chance to record where home is. So it
    // is taken as the answer rather than measured against one.
    if home.is_none() && server_origin.is_none() {
        showing.set_home_if_unset(target.to_string());
    }

    allowed
}

/// Whether a navigation belongs inside Shiver.
///
/// Pure, and separate from the state it reads, because getting this wrong is not a quiet failure:
/// a navigation Shiver wrongly calls foreign is handed to the system browser, so Shiver's own page
/// opens in a browser tab instead of in the app.
pub fn navigation_allowed(home: Option<&str>, server_origin: Option<&str>, target: &Url) -> bool {
    if let Some(origin) = server_origin {
        if is_same_origin(origin, target) {
            return true;
        }
    }

    // nothing has told Shiver where its own pages are yet, and the only navigation that can happen
    // before a server is opened is Shiver's own
    let Some(home) = home else {
        return true;
    };

    same_place(home, target)
}

/// Whether a url is where Shiver's own pages live.
fn same_place(home: &str, target: &Url) -> bool {
    Url::parse(home).is_ok_and(|home| {
        home.scheme() == target.scheme()
            && home.host_str() == target.host_str()
            // the port is part of it: a dev server and a server on the same host are not the same
            // place, and Shiver must not treat one as the other
            && home.port_or_known_default() == target.port_or_known_default()
    })
}

/// Records that the webview has landed back on Shiver's own pages.
///
/// **`show_shiver` is not the only way back.** A server's page has no IPC, so everything the bridge
/// offers that leaves the server — settings, adding a server, the direct-message list — gets there
/// by navigating (`goHome`), which calls no command and so ran none of the bookkeeping that command
/// does. `Showing::server` went on naming the server the user had left, which meant `inbox::sync`
/// never gave that server a socket: its badge stopped moving and its conversations were missing from
/// the one screen built to list every server's, for as long as the user stayed on Shiver's pages.
///
/// Called from the page-load hook, so it covers every arrival however it was reached.
pub fn landed_home(app: &AppHandle) {
    let showing = app.state::<Showing>();

    // already known, and `sync` is not free — it is called on every page load otherwise
    if showing.server().is_none() {
        return;
    }

    showing.set_server(None);

    // nothing is on screen now, so every server Shiver has a token for is worth watching
    crate::inbox::sync(app);
}

/// Whether a loaded page is one of Shiver's own rather than a server's.
pub fn is_home(app: &AppHandle, target: &Url) -> bool {
    app.state::<Showing>()
        .home()
        .is_some_and(|home| same_place(&home, target))
}

/// `null` while the user is on Sharkord's own colours, so a stock Shiver restyles nothing.
/// Decides Sharkord's own light/dark class before the page has painted anything.
///
/// **This is the second white flash, and it is not the same one `background_color` fixes.** That one
/// is the gap *between* two documents, where nothing is painting. This one is the new document
/// painting white on purpose, and it happens after Shiver's "Connecting to …" screen has already gone.
///
/// Sharkord's stylesheet is light-first. `:root` sets `--background: oklch(1 0 0)` — pure white —
/// and dark is a `.dark` class on `<html>` that its React `ThemeProvider` adds *in an effect*
/// (`components/theme-provider/index.tsx`), which cannot run until React has mounted, which in
/// `main.tsx` is after `await i18nReady`. So the page's own CSS lands first and paints white for as
/// long as that takes. Setting the webview's background cannot help: the page is painting, and it
/// is painting white.
///
/// So Shiver decides the same thing, earlier, and by exactly the same rule Sharkord will: the stored
/// preference, its `defaultTheme="dark"` when there is none, and the system for `'system'`. When
/// Sharkord's effect finally runs it removes both classes and adds the one it wanted, which is the
/// one already there — so nothing moves.
///
/// Three things worth knowing:
///
/// - **`documentElement` may not exist yet.** A document-start script runs after the global object
///   is created and before the document is parsed, so `<html>` is not reliably there. Hence the
///   observer, which costs nothing and disconnects the moment it fires.
/// - **It must be idempotent.** On Shiver's own pages this arrives twice — wry injects init scripts
///   into html served through its custom protocol *and* registers them with
///   `addDocumentStartJavaScript` — so it returns early if either class is already set. On Shiver's
///   own pages it is inert anyway; nothing in Shiver's css keys off `.dark`.
/// - **It grants the page nothing.** It is a class name on `<html>`. A page on a server's origin
///   still has no IPC, which is enforced in the core and not by what Shiver chooses to inject.
///
/// It does not carry the user's *chosen* background. That still arrives with the bridge after load,
/// so someone on a custom colour sees Sharkord's dark first and then their own — a step, but not a
/// white one, which is the thing being fixed. Baking the colour in here would mean a value frozen
/// at window-build time going stale the moment they changed it.
///
/// Degrades to today's behaviour rather than breaking: `RustWebView` only registers these when
/// `WebViewFeature.DOCUMENT_START_SCRIPT` is supported, and an old WebView simply flashes as before.
///
/// **This is not the bridge, and does not move the bridge.** The bridge still runs after page load
/// for the reasons in this module's own note and in SECURITY-REVIEW.md §11 — it needs the page's dom
/// and Sharkord's store to exist. This is a few lines that must beat the first paint and touch
/// nothing else.
pub const THEME_BEFORE_FIRST_PAINT: &str = r#"
(function () {
  var KEY = 'vite-ui-theme';
  var DEFAULT = 'dark';

  function chosen() {
    var stored = null;

    try {
      stored = localStorage.getItem(KEY);
    } catch (error) {
      // a page can refuse storage; the default is the answer then
    }

    var theme = stored === 'dark' || stored === 'light' || stored === 'system' ? stored : DEFAULT;

    if (theme !== 'system') return theme;

    try {
      return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    } catch (error) {
      return DEFAULT;
    }
  }

  function apply() {
    var root = document.documentElement;

    if (!root) return false;
    // Sharkord already decided, or Shiver already did on an earlier run of this script
    if (root.classList.contains('dark') || root.classList.contains('light')) return true;

    root.classList.add(chosen());

    return true;
  }

  try {
    if (apply()) return;

    var observer = new MutationObserver(function () {
      if (apply()) observer.disconnect();
    });

    // childList on the document alone: `<html>` is its direct child, so there is nothing deeper
    // worth watching, and this fires once and disconnects.
    observer.observe(document, { childList: true });
  } catch (error) {
    // never let this take a server's client down with it; the cost of failing is a white frame
  }
})();
"#;

/// The colour the webview itself paints when no page is painting.
///
/// **This is what removes the white flash between servers.** Switching servers on mobile is two
/// navigations, not one — the rail inside a server's page cannot call Shiver, so tapping a tile goes
/// to Shiver's own page carrying a fragment, and Shiver then navigates on to the server. For each of
/// those the old document is torn down before the new one paints, and what shows in the gap is the
/// webview's own background, which Android defaults to **white**. On a client that is black
/// everywhere that reads as a flash of the phone's torch.
///
/// The window background was already dark (`themes.xml`, `@color/shiver_background`) but that is only
/// the strip behind the system bars — the webview is inset and draws over the rest.
///
/// Taken from the user's own background colour rather than hard-coded, so someone who has chosen a
/// colour flashes *their* colour, which is to say does not flash at all. `MainActivity` sets
/// Sharkord's default on the webview the moment it exists, which covers the instant before this
/// runs; this then refines it, and `update_settings` keeps it in step.
pub fn background_color(settings: &Settings) -> tauri::window::Color {
    parse_hex(&settings.theme_color).unwrap_or(tauri::window::Color(0x0a, 0x0a, 0x0a, 0xff))
}

/// `#rgb` or `#rrggbb` to a colour, opaque. `None` for anything else, so a malformed stored value
/// falls back rather than painting something arbitrary.
fn parse_hex(value: &str) -> Option<tauri::window::Color> {
    let digits = value.strip_prefix('#')?;

    let (r, g, b) = match digits.len() {
        // `#abc` is shorthand for `#aabbcc`, so each digit is doubled rather than shifted
        3 => {
            let mut nibbles = digits
                .chars()
                .map(|digit| digit.to_digit(16).map(|value| (value * 17) as u8).ok_or(()));

            (
                nibbles.next()?.ok()?,
                nibbles.next()?.ok()?,
                nibbles.next()?.ok()?,
            )
        }
        6 => (
            u8::from_str_radix(&digits[0..2], 16).ok()?,
            u8::from_str_radix(&digits[2..4], 16).ok()?,
            u8::from_str_radix(&digits[4..6], 16).ok()?,
        ),
        _ => return None,
    };

    Some(tauri::window::Color(r, g, b, 0xff))
}

fn theme_payload(settings: &Settings) -> serde_json::Value {
    if settings.uses_default_colors() {
        return serde_json::Value::Null;
    }

    json!({
        "themeColor": settings.theme_color,
        "accentColor": settings.accent_color,
        "textColor": settings.text_color,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("a url")
    }

    /// What the pre-paint script agrees with Sharkord about.
    ///
    /// It is a *copy* of another program's decision, which is the fragile kind of coupling — the
    /// key, the default and the class names all have to match `components/theme-provider/index.tsx`
    /// and `index.css`. This does not prove the script works; the logic was exercised against a
    /// fake dom separately. It catches the thing that will actually go wrong, which is someone
    /// editing the script or a Sharkord upgrade moving one of these and nothing saying so.
    ///
    /// If this fails after a Sharkord upgrade, re-read that theme provider before changing it.
    #[test]
    fn the_pre_paint_script_matches_sharkords_own_theme_rule() {
        let script = THEME_BEFORE_FIRST_PAINT;

        // the storage key Sharkord reads (`LocalStorageKey.VITE_UI_THEME`)
        assert!(script.contains("'vite-ui-theme'"));
        // its `defaultTheme="dark"` in main.tsx, which is what an absent preference means
        assert!(script.contains("var DEFAULT = 'dark'"));
        // the two class names `index.css` keys off, via `@custom-variant dark (&:is(.dark *))`
        assert!(script.contains("'dark'") && script.contains("'light'"));
        // and the third value the provider accepts, which resolves against the device
        assert!(script.contains("'system'"));
        assert!(script.contains("prefers-color-scheme: dark"));

        // idempotent: it arrives twice on Shiver's own pages, and must not fight a decision already
        // made — by Sharkord or by an earlier run of itself
        assert!(script.contains("classList.contains('dark')"));
        assert!(script.contains("classList.contains('light')"));

        // `<html>` is not reliably there when a document-start script runs
        assert!(script.contains("MutationObserver"));

        // it injects a class and nothing else: no fetch, no ipc, no storage writes
        for forbidden in [
            "__TAURI",
            "invoke",
            "fetch(",
            "XMLHttpRequest",
            "setItem",
            "eval(",
        ] {
            assert!(
                !script.contains(forbidden),
                "the pre-paint script must not contain {forbidden}"
            );
        }
    }

    /// The colour that shows between two pages, which is the whole of the white-flash fix — so the
    /// fallback matters as much as the parse: a stored value this cannot read must still come back
    /// dark, never white.
    #[test]
    fn a_colour_is_read_from_its_hex() {
        assert_eq!(
            parse_hex("#0a0a0a"),
            Some(tauri::window::Color(10, 10, 10, 255))
        );
        assert_eq!(
            parse_hex("#FFFFFF"),
            Some(tauri::window::Color(255, 255, 255, 255))
        );
        // shorthand doubles each digit rather than shifting it, so #abc is #aabbcc
        assert_eq!(
            parse_hex("#abc"),
            Some(tauri::window::Color(0xaa, 0xbb, 0xcc, 255))
        );
    }

    #[test]
    fn an_unreadable_colour_is_no_colour() {
        for bad in ["", "0a0a0a", "#", "#12", "#12345", "#zzzzzz", "#0a0a0a0a"] {
            assert_eq!(parse_hex(bad), None, "{bad} should not parse");
        }
    }

    /// Sharkord's own background, never the platform's white, whatever is in the settings file.
    #[test]
    fn an_unreadable_setting_still_paints_dark() {
        let settings = Settings {
            theme_color: "rgb(255, 0, 0)".into(),
            ..Default::default()
        };

        assert_eq!(
            background_color(&settings),
            tauri::window::Color(0x0a, 0x0a, 0x0a, 0xff)
        );
    }

    /// The opening navigation happens while the window is still being built, before anything knows
    /// where home is. Calling it foreign is what put Shiver's own page in the system browser.
    #[test]
    fn the_first_navigation_is_shivers_own_page() {
        assert!(navigation_allowed(
            None,
            None,
            &url("http://localhost:1421/")
        ));
    }

    #[test]
    fn shivers_own_pages_are_allowed_once_home_is_known() {
        let home = Some("http://localhost:1421/");

        assert!(navigation_allowed(
            home,
            None,
            &url("http://localhost:1421/")
        ));
        assert!(navigation_allowed(
            home,
            None,
            &url("http://localhost:1421/index.html")
        ));
    }

    /// The exact failure: home recorded as the app URL while the webview navigates to the dev
    /// server. They are different places, so Shiver called its own page foreign — which is why home
    /// is written once, by whichever side sees it first, and never overwritten.
    #[test]
    fn a_home_from_the_wrong_side_rejects_the_dev_server() {
        assert!(!navigation_allowed(
            Some("http://tauri.localhost/"),
            None,
            &url("http://localhost:1421/")
        ));
    }

    #[test]
    fn the_open_server_is_allowed() {
        let home = Some("http://localhost:1421/");

        assert!(navigation_allowed(
            home,
            Some("https://chat.example.com"),
            &url("https://chat.example.com/channels/1")
        ));
    }

    /// Origin pinning: a link out of Sharkord belongs in the browser, so a server cannot move the
    /// one webview Shiver has somewhere it still labels as that server.
    #[test]
    fn a_link_out_of_the_server_is_not_allowed() {
        let home = Some("http://localhost:1421/");

        assert!(!navigation_allowed(
            home,
            Some("https://chat.example.com"),
            &url("https://elsewhere.example.com/")
        ));
    }

    #[test]
    fn going_back_to_shiver_from_a_server_is_allowed() {
        assert!(navigation_allowed(
            Some("http://localhost:1421/"),
            Some("https://chat.example.com"),
            &url("http://localhost:1421/")
        ));
    }

    /// Same host, different port is a different place — a dev server is not the server next door.
    #[test]
    fn the_port_is_part_of_home() {
        assert!(!navigation_allowed(
            Some("http://localhost:1421/"),
            None,
            &url("http://localhost:1420/")
        ));
    }

    fn entry(id: &str, name: &str, origin: &str, position: i32) -> ServerEntry {
        ServerEntry {
            accept_any_size: false,
            id: id.into(),
            origin: origin.into(),
            server_id: None,
            name: name.into(),
            icon_url: Some(format!("{origin}/public/logo.png")),
            icon_data: None,
            identity: Some("someone".into()),
            account_label: Some("someone".into()),
            folder_id: None,
            position,
        }
    }

    /// The one place cross-server data enters a server's page, so what it carries is asserted
    /// rather than trusted: a hostile server may learn its neighbours' names, never their
    /// addresses.
    #[test]
    fn the_rail_payload_carries_no_addresses() {
        let servers = vec![
            entry("a", "Alpha", "https://alpha.example.com", 0),
            entry("b", "Beta", "https://beta.example.com", 1),
        ];

        let payload = rail_payload(&servers, &Default::default(), &[]).to_string();

        assert!(payload.contains("Alpha") && payload.contains("Beta"));
        assert!(!payload.contains("alpha.example.com"));
        assert!(!payload.contains("beta.example.com"));
        assert!(!payload.contains("someone"));
    }

    #[test]
    fn the_rail_is_ordered_by_position() {
        let servers = vec![
            entry("a", "Alpha", "https://alpha.example.com", 7),
            entry("b", "Beta", "https://beta.example.com", 2),
        ];

        let payload = rail_payload(&servers, &Default::default(), &[]);

        assert_eq!(payload[0]["id"], "b");
        assert_eq!(payload[1]["id"], "a");
    }

    /// Opening the dm list is something the user asked for once, on arrival. Left set it would
    /// re-open on every reload and every navigation within the server.
    #[test]
    fn a_pending_dm_request_is_consumed_once() {
        let showing = Showing::default();

        showing.set_pending_dms(true);

        assert!(showing.take_pending_dms());
        assert!(!showing.take_pending_dms());
    }

    #[test]
    fn home_is_written_once_and_never_overwritten() {
        let showing = Showing::default();

        showing.set_home_if_unset("http://localhost:1421/".into());
        showing.set_home_if_unset("http://tauri.localhost/".into());

        assert_eq!(showing.home().as_deref(), Some("http://localhost:1421/"));
    }
}
