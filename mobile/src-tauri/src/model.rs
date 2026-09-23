//! The registry saved in `servers.json`. Nothing secret lives here; credentials are in the
//! encrypted store.

use serde::{Deserialize, Serialize};

pub use shiver_core::{
    is_same_origin,
    model::{
        sanitised_color, sanitised_optional_color, Folder, MutedChannel, DEFAULT_ACCENT_COLOR,
        DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME,
    },
    normalize_origin,
    probe::ServerInfo,
};

/// One rail entry; `id` identifies it everywhere, never the origin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub id: String,
    pub origin: String,
    #[serde(default)]
    pub server_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    /// the logo as a `data:` uri: the rail is drawn inside other servers' pages, and a url would
    /// tell them where this server lives
    #[serde(default)]
    pub icon_data: Option<String>,
    /// raises (never removes) the websocket message size limit for a server the user trusts
    #[serde(default)]
    pub accept_any_size: bool,
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub folder_id: Option<String>,
    pub position: i32,
    /// The UnifiedPush instance token. Random and never shown to any page, since broadcasts that
    /// carry a known token are trusted; the entry id cannot serve, because every page sees those.
    #[serde(default)]
    pub push_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme_color: String,
    pub accent_color: String,
    #[serde(default)]
    pub text_color: Option<String>,
    #[serde(default = "default_sound_volume")]
    pub sound_volume: u16,
    #[serde(default = "default_true")]
    pub minimise_attachments: bool,
    #[serde(default)]
    pub last_server_id: Option<String>,
    /// entries allowed to wake the phone over UnifiedPush
    #[serde(default)]
    pub push_servers: Vec<String>,
    #[serde(default)]
    pub skipped_update: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_sound_volume() -> u16 {
    100
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_color: DEFAULT_THEME_COLOR.into(),
            accent_color: DEFAULT_ACCENT_COLOR.into(),
            text_color: None,
            sound_volume: default_sound_volume(),
            minimise_attachments: true,
            last_server_id: None,
            push_servers: Vec::new(),
            skipped_update: None,
        }
    }
}

impl Settings {
    pub fn uses_default_colors(&self) -> bool {
        shiver_core::model::uses_default_colors(
            &self.theme_color,
            &self.accent_color,
            self.text_color.as_deref(),
        )
    }

    /// Every field held to a valid, safe value; applied on the way in and out of the file.
    pub fn sanitised(mut self) -> Self {
        self.theme_color = sanitised_color(&self.theme_color, DEFAULT_THEME_COLOR);
        self.accent_color = sanitised_color(&self.accent_color, DEFAULT_ACCENT_COLOR);
        self.text_color = sanitised_optional_color(self.text_color.as_deref());
        self.sound_volume = self.sound_volume.min(MAX_SOUND_VOLUME);
        self
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

impl Registry {
    pub fn server(&self, id: &str) -> Option<&ServerEntry> {
        self.servers.iter().find(|server| server.id == id)
    }

    pub fn server_mut(&mut self, id: &str) -> Option<&mut ServerEntry> {
        self.servers.iter_mut().find(|server| server.id == id)
    }

    pub fn folder_mut(&mut self, id: &str) -> Option<&mut Folder> {
        self.folders.iter_mut().find(|folder| folder.id == id)
    }

    pub fn muted_for(&self, entry_id: &str) -> Vec<i64> {
        shiver_core::model::muted_for(&self.muted, entry_id)
    }

    pub fn set_muted_for(
        &mut self,
        entry_id: &str,
        channels: impl IntoIterator<Item = i64>,
    ) -> Vec<i64> {
        shiver_core::model::set_muted_for(&mut self.muted, entry_id, channels)
    }

    /// The next free top-level position (servers and folders share one space).
    pub fn next_position(&self) -> i32 {
        self.servers
            .iter()
            .filter(|server| server.folder_id.is_none())
            .map(|server| server.position)
            .chain(self.folders.iter().map(|folder| folder.position))
            .max()
            .map_or(0, |max| max + 1)
    }

    /// The entry a push token belongs to.
    pub fn entry_for_push_token(&self, token: &str) -> Option<&ServerEntry> {
        self.servers
            .iter()
            .find(|server| server.push_token.as_deref() == Some(token))
    }

    /// Gives every entry a push token; returns the ids of entries that lacked one.
    pub fn ensure_push_tokens(&mut self) -> Vec<String> {
        self.servers
            .iter_mut()
            .filter(|server| server.push_token.is_none())
            .map(|server| {
                server.push_token = Some(uuid::Uuid::new_v4().simple().to_string());
                server.id.clone()
            })
            .collect()
    }
}

/// Dissolves folders holding fewer than two servers: their servers take the folder's place at the
/// top level, which is then renumbered 0, 1, 2… Returns whether anything changed.
pub fn prune_folders(registry: &mut Registry) -> bool {
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

    items.sort_by_key(|(position, _, _)| *position);

    for (index, (_, is_folder, id)) in items.iter().enumerate() {
        if *is_folder {
            if let Some(folder) = registry.folder_mut(id) {
                folder.position = index as i32;
            }
        } else if let Some(server) = registry.server_mut(id) {
            server.position = index as i32;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

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
            id: id.into(),
            name: id.into(),
            origin: format!("https://{id}.example.com"),
            folder_id: folder.map(str::to_string),
            ..Default::default()
        }
    }
}
