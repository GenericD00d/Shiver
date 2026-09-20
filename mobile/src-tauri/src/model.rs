use serde::{Deserialize, Serialize};

/// One entry in the server rail. The same `origin` may appear more than once so a user can be
/// signed in to one server under several accounts; `id` is what identifies the session everywhere
/// else in Shiver, never the origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub id: String,
    pub origin: String,
    /// `serverId` as reported by GET /info. Two entries sharing it are the same server.
    #[serde(default)]
    pub server_id: Option<String>,
    pub name: String,
    /// absolute url of the server logo, resolved when the entry was added or last refreshed
    #[serde(default)]
    pub icon_url: Option<String>,
    /// the logo itself, as a `data:` uri.
    ///
    /// Shiver's rail is drawn *inside* a server's page on mobile, because Android gives a window one
    /// webview and there is nowhere else to put it. A rail of `<img src>` pointing at other servers
    /// would hand the page every other server's address and make it fetch from each one. The bytes
    /// travel instead of the address, so a server can see what its neighbours look like and never
    /// where they are.
    #[serde(default)]
    pub icon_data: Option<String>,
    /// Whether this server may send Shiver larger messages than the default limit allows.
    ///
    /// Shiver bounds what a server may send over its socket, because a socket per server on a phone
    /// is a lot of memory to hand to strangers. The bound is a guess about *other people's*
    /// servers, though, and it is wrong for a big one — so this is the answer for a server the user
    /// actually trusts, usually their own.
    ///
    /// **It raises the ceiling; it does not remove it.** This used to say "no limit, and the memory
    /// is theirs to spend", which has not been true since `MAX_FRAME_TRUSTED` — trusting a server
    /// gives it eight times the room and still bounds it, and `sharkord-client` has a test asserting
    /// that it can never become unbounded. An unbounded socket lets a server grow the read buffer as
    /// far as it cares to transmit, which on a phone is an app kill that takes the notifications
    /// with it.
    ///
    /// Off unless somebody says otherwise, and said per server rather than once for all of them,
    /// because trusting one server is not trusting the next.
    #[serde(default)]
    pub accept_any_size: bool,
    /// the username, kept only so the rail can tell two accounts on one server apart. The mobile
    /// client never signs in itself — there is no keychain on Android — so this is a label here,
    /// not a credential.
    #[serde(default)]
    pub identity: Option<String>,
    /// shown under the icon when one origin is added more than once, so the accounts are tellable apart
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub folder_id: Option<String>,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub position: i32,
    #[serde(default)]
    pub expanded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// user-owned theme colours, applied to Shiver's own chrome and handed to the bridge
    pub theme_color: String,
    pub accent_color: String,
    /// The colour text is drawn in, or `None` to take it from the background.
    ///
    /// Absent by default, and absent is not the same as a colour that happens to match: on a light
    /// background Shiver picks dark text and on a dark one light text, and a stored colour would keep
    /// whatever was chosen even after the background moved out from under it. Someone who wants
    /// their own colour says so; everyone else keeps text that follows what they can read.
    #[serde(default)]
    pub text_color: Option<String>,
    /// How loud each server's own sounds are, as a percentage.
    ///
    /// Above 100 is the point of it: Sharkord synthesises its sounds through Web Audio, and an
    /// `<audio>` element cannot be turned past 1.0 — but a gain node can, so a quiet notification
    /// can be made to carry over somebody talking. The phone's own volume keys move everything at
    /// once; this moves the ping relative to the call, which is what they cannot do. Voice is
    /// untouched: it never reaches the speakers through this path.
    #[serde(default = "default_sound_volume")]
    pub sound_volume: u16,
    /// Shrink the file card under a picture down to its icon.
    ///
    /// Sharkord renders an image attachment twice — as the picture, and again as a card below it
    /// carrying the filename and the size. On by default: the second copy repeats what the first
    /// already shows, and on a phone's width that is most of the screen.
    #[serde(default = "default_true")]
    pub minimise_attachments: bool,
    /// the server Shiver reopens on next launch
    #[serde(default)]
    pub last_server_id: Option<String>,
    /// Which servers may wake the phone while Shiver is closed, by rail entry id.
    ///
    /// Named rather than counted, and empty by default. Choosing a distributor used to register
    /// every server at once, which hands each of them an address to reach the phone on without
    /// anyone being asked about that server in particular — and a server nobody wants to be woken
    /// by is the common case with more than one or two. Each entry here is a deliberate answer.
    #[serde(default)]
    pub push_servers: Vec<String>,
    /// A version the user asked not to be told about again.
    ///
    /// One version, not a list: the question is only ever "is the newest one the one I turned
    /// down", and a release after it is news again. Skipping is not the same as refusing updates —
    /// there is no setting for that, and this quietly becoming one would be a poor way to get it.
    #[serde(default)]
    pub skipped_update: Option<String>,
}

/// Sharkord's own dark theme, so Shiver out of the box looks like Sharkord out of the box.
/// These are `--background` (oklch(0.145 0 0)) and `--primary` (oklch(0.922 0 0)) from
/// `apps/client/src/index.css`, converted to hex.
pub const DEFAULT_THEME_COLOR: &str = "#0a0a0a";
pub const DEFAULT_ACCENT_COLOR: &str = "#e5e5e5";

fn default_true() -> bool {
    true
}

/// Unchanged from what Sharkord already does, so an existing install sounds the same.
fn default_sound_volume() -> u16 {
    100
}

/// Silent, through to loud enough to hear over a call. Clamped rather than trusted: this is a
/// number a page could put in the settings file, and a gain of 400 is a way to hurt somebody.
pub const MAX_SOUND_VOLUME: u16 = 250;

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_color: DEFAULT_THEME_COLOR.into(),
            accent_color: DEFAULT_ACCENT_COLOR.into(),
            text_color: None,
            sound_volume: 100,
            minimise_attachments: true,
            last_server_id: None,
            push_servers: Vec::new(),
            skipped_update: None,
        }
    }
}

/// A colour Shiver is willing to put in a stylesheet, or the default in its place.
///
/// These are plain strings, they arrive from `update_settings`, and they live in a file a person
/// can edit. The bridge sets them through the CSSOM rather than building a stylesheet as a string,
/// so a value containing `}` is rejected by the parser rather than closing a rule block — but that
/// is one mechanism holding the line on its own, in the half of the app that cannot be tested on a
/// desktop. Desktop checks the value as well as setting it safely; so does this now.
///
/// Six hex digits with a leading `#`, which is all the colour input can produce and all either
/// side knows how to read.
pub fn sanitised_color(value: &str, fallback: &str) -> String {
    if is_hex_color(value) {
        return value.to_ascii_lowercase();
    }

    eprintln!("[shiver] '{value}' is not a colour Shiver will use, falling back to {fallback}");

    fallback.to_string()
}

/// The same rule, for the optional text colour: kept when it is a colour, dropped when it is not.
pub fn sanitised_optional_color(value: Option<&str>) -> Option<String> {
    match value {
        Some(value) if is_hex_color(value) => Some(value.to_ascii_lowercase()),
        Some(value) => {
            eprintln!("[shiver] '{value}' is not a colour Shiver will use, so text follows the background");

            None
        }
        None => None,
    }
}

fn is_hex_color(value: &str) -> bool {
    let Some(digits) = value.strip_prefix('#') else {
        return false;
    };

    digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl Settings {
    /// True while the user has not picked their own colours. Shiver injects no css into a server's
    /// page in that case, so a stock server looks exactly as its own client intends.
    pub fn uses_default_colors(&self) -> bool {
        self.theme_color.eq_ignore_ascii_case(DEFAULT_THEME_COLOR)
            && self.accent_color.eq_ignore_ascii_case(DEFAULT_ACCENT_COLOR)
            && self.text_color.is_none()
    }

    /// Every colour in this struct, held to what is safe to put in a stylesheet, and the volume
    /// held to what Shiver will actually play.
    ///
    /// Applied on the way in from the settings screen and on the way out of the file, so a value
    /// that was already stored by an older build cannot reach the bridge either.
    pub fn sanitised(mut self) -> Self {
        self.theme_color = sanitised_color(&self.theme_color, DEFAULT_THEME_COLOR);
        self.accent_color = sanitised_color(&self.accent_color, DEFAULT_ACCENT_COLOR);
        self.text_color = sanitised_optional_color(self.text_color.as_deref());
        self.sound_volume = self.sound_volume.min(MAX_SOUND_VOLUME);

        self
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn a_colour_that_could_close_a_rule_block_is_refused() {
        let settings = Settings {
            theme_color: "#0a0a0a} html { display: none } .x {".into(),
            accent_color: "red".into(),
            text_color: Some("#ff0000; content: 'x'".into()),
            sound_volume: 9999,
            ..Settings::default()
        }
        .sanitised();

        assert_eq!(settings.theme_color, DEFAULT_THEME_COLOR);
        assert_eq!(settings.accent_color, DEFAULT_ACCENT_COLOR);
        assert_eq!(settings.text_color, None);
        assert_eq!(settings.sound_volume, MAX_SOUND_VOLUME);
    }

    #[test]
    fn a_real_colour_survives_and_is_lowercased() {
        let settings = Settings {
            theme_color: "#AABBCC".into(),
            text_color: Some("#FFFFFF".into()),
            ..Settings::default()
        }
        .sanitised();

        assert_eq!(settings.theme_color, "#aabbcc");
        assert_eq!(settings.text_color.as_deref(), Some("#ffffff"));
    }
}

/// A channel the user has muted, scoped to one rail entry so muting a channel on one account never
/// silences the same channel viewed from another.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutedChannel {
    pub entry_id: String,
    pub channel_id: i64,
}

/// Dissolves any folder that is no longer holding a group.
///
/// A folder exists to gather servers together, so one holding a single server is a worse-looking
/// server and one holding none is nothing at all. Rather than leave either lying around for the
/// user to tidy up, a folder that falls below two members hands its server back to the top level
/// and goes away — which is also what makes dragging the second-to-last server out of a folder do
/// the obvious thing rather than leaving a husk behind.
///
/// Returns whether anything changed, so callers can avoid writing a registry that is already right.
pub fn prune_folders(registry: &mut Registry) -> bool {
    // the id of each folder that is going, and the place in the rail it is leaving behind
    let doomed: Vec<(String, i32)> = registry
        .folders
        .iter()
        .filter(|folder| {
            registry
                .servers
                .iter()
                .filter(|server| server.folder_id.as_deref() == Some(folder.id.as_str()))
                .count()
                < 2
        })
        .map(|folder| (folder.id.clone(), folder.position))
        .collect();

    if doomed.is_empty() {
        return false;
    }

    for server in registry.servers.iter_mut() {
        // Out at the folder's own place in the rail, not at the position it held inside it. A
        // server freed this way should be found where its folder was; an inside-the-folder position
        // means nothing at the top level and is a small number that would drag the server to the
        // front, onto whoever is already there.
        if let Some((_, place)) = server
            .folder_id
            .as_ref()
            .and_then(|id| doomed.iter().find(|(doomed, _)| doomed == id))
        {
            server.position = *place;
            server.folder_id = None;
        }
    }

    registry
        .folders
        .retain(|folder| !doomed.iter().any(|(id, _)| id == &folder.id));

    renumber_top_level(registry);

    true
}

/// Gives the top level of the rail the positions 0, 1, 2 … in the order it is already in.
///
/// A position is only ever read as an order, so the numbers themselves do not matter — but two
/// items sharing one do, because then the order is whatever the sort happens to make of the tie.
/// Dissolving a folder is where ties come from, so they are spent here.
fn renumber_top_level(registry: &mut Registry) {
    let mut items: Vec<(i32, bool, String)> = registry
        .servers
        .iter()
        .filter(|server| server.folder_id.is_none())
        .map(|server| (server.position, false, server.id.clone()))
        .chain(
            registry
                .folders
                .iter()
                .map(|folder| (folder.position, true, folder.id.clone())),
        )
        .collect();

    // stable, so anything already tied keeps the order it had rather than being shuffled
    items.sort_by_key(|(position, _, _)| *position);

    for (index, (_, is_folder, id)) in items.iter().enumerate() {
        let position = index as i32;

        if *is_folder {
            if let Some(folder) = registry.folders.iter_mut().find(|folder| &folder.id == id) {
                folder.position = position;
            }
        } else if let Some(server) = registry.servers.iter_mut().find(|server| &server.id == id) {
            server.position = position;
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub muted: Vec<MutedChannel>,
}

/// What GET /info exposes to a client that has not signed in yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub origin: String,
    pub server_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
}

/// The origin rules, which live in `shared/shiver-core` so there is one copy rather than two.
///
/// These were byte-identical in both clients, tests included, and `is_same_origin` is the whole of
/// Shiver's webview pinning — the single comparison that decides whether a page may navigate
/// somewhere. Re-exported rather than moved out of sight, so every existing caller reads the same.
pub use shiver_core::{is_same_origin, normalize_origin};

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder earns its place by holding a group. One server is a worse-looking server and none
    /// is nothing, so both dissolve and the servers come back to the top level rather than being
    /// lost with the folder.
    #[test]
    fn a_folder_holding_fewer_than_two_servers_dissolves() {
        let mut registry = Registry {
            folders: vec![
                Folder {
                    id: "keep".into(),
                    name: "Keep".into(),
                    position: 0,
                    expanded: true,
                },
                Folder {
                    id: "lonely".into(),
                    name: "Lonely".into(),
                    position: 1,
                    expanded: true,
                },
                Folder {
                    id: "empty".into(),
                    name: "Empty".into(),
                    position: 2,
                    expanded: true,
                },
            ],
            servers: vec![
                folder_member("a", Some("keep")),
                folder_member("b", Some("keep")),
                folder_member("c", Some("lonely")),
                folder_member("d", None),
            ],
            ..Default::default()
        };

        assert!(prune_folders(&mut registry));

        let left: Vec<&str> = registry.folders.iter().map(|f| f.id.as_str()).collect();

        assert_eq!(left, vec!["keep"]);

        // the one that was on its own is still here, just no longer in a folder
        let c = registry.servers.iter().find(|s| s.id == "c").unwrap();

        assert_eq!(c.folder_id, None);
        assert_eq!(registry.servers.len(), 4);
    }

    /// Idempotent, because it runs on a timer and on every membership change.
    #[test]
    fn pruning_a_tidy_rail_changes_nothing() {
        let mut registry = Registry {
            folders: vec![Folder {
                id: "keep".into(),
                name: "Keep".into(),
                position: 0,
                expanded: true,
            }],
            servers: vec![
                folder_member("a", Some("keep")),
                folder_member("b", Some("keep")),
            ],
            ..Default::default()
        };

        assert!(!prune_folders(&mut registry));
        assert_eq!(registry.folders.len(), 1);
    }

    /// A folder dissolving must not throw the server it held to the front of the rail. Inside a
    /// folder the positions start at zero again, so the freed server takes the folder's place
    /// instead and the top level comes out numbered 0, 1, 2 with nothing tied.
    #[test]
    fn a_freed_server_lands_where_its_folder_was() {
        let mut registry = Registry {
            folders: vec![Folder {
                id: "lonely".into(),
                name: "Lonely".into(),
                position: 1,
                expanded: true,
            }],
            servers: vec![
                at("first", None, 0),
                // inside the folder, so this zero says nothing about the top level
                at("freed", Some("lonely"), 0),
                at("last", None, 2),
            ],
            ..Default::default()
        };

        assert!(prune_folders(&mut registry));

        let mut order: Vec<(&str, i32)> = registry
            .servers
            .iter()
            .map(|server| (server.id.as_str(), server.position))
            .collect();

        order.sort_by_key(|(_, position)| *position);

        assert_eq!(order, vec![("first", 0), ("freed", 1), ("last", 2)]);
    }

    fn at(id: &str, folder: Option<&str>, position: i32) -> ServerEntry {
        ServerEntry {
            position,
            ..folder_member(id, folder)
        }
    }

    fn folder_member(id: &str, folder: Option<&str>) -> ServerEntry {
        ServerEntry {
            accept_any_size: false,
            id: id.into(),
            name: id.into(),
            origin: format!("https://{id}.example.com"),
            icon_url: None,
            icon_data: None,
            server_id: None,
            identity: None,
            account_label: None,
            folder_id: folder.map(str::to_string),
            position: 0,
        }
    }
}
