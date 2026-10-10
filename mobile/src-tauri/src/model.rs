//! The registry saved in `servers.json`. Nothing secret lives here; credentials are in the
//! encrypted store.

use serde::{Deserialize, Serialize};

use shiver_core::model::{default_sound_volume, default_true};
pub use shiver_core::{
    is_same_origin,
    model::{
        sanitised_color, sanitised_optional_color, Folder, MutedChannel, DEFAULT_ACCENT_COLOR,
        DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME,
    },
    normalize_origin,
    probe::ServerInfo,
};

shiver_core::registry!(Registry, ServerEntry);

/// One rail entry; `id` identifies it everywhere, never the origin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub id: String,
    pub origin: String,
    pub name: String,
    /// where the logo is fetched from; Shiver's page draws the copy in `icons`
    #[serde(default)]
    pub icon_url: Option<String>,
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
    /// endpoints this server should forget, handed to its page on the next load
    #[serde(default)]
    pub retired_push_endpoints: Vec<String>,
    /// which of its messages notify
    #[serde(default)]
    pub notify: shiver_core::model::NotifyLevel,
}

/// Retired endpoints kept per server. A server without the plugin never confirms clearing one, so
/// without a bound each switch would add one for good; the oldest go first.
const MAX_RETIRED_ENDPOINTS: usize = 10;

impl ServerEntry {
    /// Marks an endpoint for the server's page to clear with the plugin (once, newest last, bounded).
    pub fn retire_push_endpoint(&mut self, endpoint: String) {
        if self.retired_push_endpoints.contains(&endpoint) {
            return;
        }

        self.retired_push_endpoints.push(endpoint);

        let excess = self
            .retired_push_endpoints
            .len()
            .saturating_sub(MAX_RETIRED_ENDPOINTS);

        self.retired_push_endpoints.drain(..excess);
    }
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
    /// sites whose links open without asking, each from one server's pages (`shiver_core::links`);
    /// replaces `trustedLinkSites` (trusted from every server), which is dropped
    #[serde(default)]
    pub trusted_links: Vec<shiver_core::links::TrustedLink>,
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
            trusted_links: Vec::new(),
        }
    }
}

impl Settings {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_endpoints_are_kept_once_and_bounded() {
        let mut server = ServerEntry::default();

        server.retire_push_endpoint("https://a".into());
        server.retire_push_endpoint("https://a".into());
        assert_eq!(server.retired_push_endpoints, ["https://a"]);

        for index in 0..MAX_RETIRED_ENDPOINTS {
            server.retire_push_endpoint(format!("https://{index}"));
        }

        assert_eq!(server.retired_push_endpoints.len(), MAX_RETIRED_ENDPOINTS);
        assert_eq!(server.retired_push_endpoints[0], "https://0");
    }
}
