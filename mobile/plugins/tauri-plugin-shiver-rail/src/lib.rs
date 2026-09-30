//! Shiver's quick rail on Android (a no-op elsewhere).
//!
//! Android has one app webview, and Shiver's full rail is on its own page, so reaching it used to
//! unload the server's page; coming back loaded it again, and each load joins the server, which
//! Sharkord allows only a few times a minute. This rail is drawn over the page instead, which keeps
//! running (and connected) behind it, in a WebView of the plugin's own: the rail both clients draw
//! (`shared/web/components/Rail.tsx`), built into the plugin's assets (`mobile/rail`). A server's page
//! never sees it: the two share no script world, and the rail's WebView loads nothing but its own
//! page and talks only to the plugin.
//!
//! The core hands it what to draw ([`RailView`]) and hears back what the user chose ([`RailEvent`]).
//! Logos travel once: a tile names its logo by a key, and carries the logo itself only when the
//! core has not sent that key before; the rail reports the keys it lacks.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Plugin(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One server's tile.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub id: String,
    pub name: String,
    pub folder_id: Option<String>,
    /// its place at the top level, or within its folder
    pub position: i32,
    pub unread: u32,
    /// names the logo, when there is one
    pub icon_key: Option<String>,
    /// the logo as a `data:` uri, sent only with a key the rail has not been given yet
    pub icon: Option<String>,
}

/// A folder, placed among the top-level servers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RailFolder {
    pub id: String,
    pub name: String,
    pub position: i32,
    pub expanded: bool,
}

/// Everything the rail draws, and the user's colours to draw it in (`#rrggbb`; no text colour is
/// the automatic one). The page orders it as the rail on Shiver's own page does.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RailView {
    pub theme_color: String,
    pub accent_color: String,
    pub text_color: Option<String>,
    /// the server whose page is behind the rail
    pub current: Option<String>,
    pub servers: Vec<Tile>,
    pub folders: Vec<RailFolder>,
}

/// What the user did with the rail. Each but `Folder` has already closed it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RailEvent {
    /// open another server
    Open {
        #[serde(rename = "entryId")]
        entry_id: String,
    },
    /// Shiver's direct messages
    Dms,
    /// Shiver's screen for adding a server
    Add,
    /// Shiver's settings
    Settings,
    /// back to the page behind
    Closed,
    /// a folder opened or shut; the rail stays up
    Folder {
        #[serde(rename = "folderId")]
        folder_id: String,
        expanded: bool,
    },
}

#[derive(Default, Deserialize)]
struct Drawn {
    /// entry ids whose logo key the rail has not been given
    #[serde(default)]
    missing: Vec<String>,
}

pub struct Rail<R: Runtime> {
    #[cfg(target_os = "android")]
    handle: tauri::plugin::PluginHandle<R>,
    /// `fn() -> R` keeps the state `Send + Sync` whatever `R` is
    #[cfg(not(target_os = "android"))]
    marker: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> Rail<R> {
    #[cfg(target_os = "android")]
    fn call<T: DeserializeOwned + Default>(&self, command: &str, payload: Value) -> Result<T> {
        self.handle
            .run_mobile_plugin(command, payload)
            .map_err(|error| Error::Plugin(error.to_string()))
    }

    #[cfg(not(target_os = "android"))]
    fn call<T: DeserializeOwned + Default>(&self, _command: &str, _payload: Value) -> Result<T> {
        Ok(T::default())
    }

    /// Draws the rail and slides it in (or redraws it, if it is up). Returns the entries whose
    /// logo the rail lacks. Blocks until the UI thread has drawn it, so never call it from there.
    pub fn show(&self, view: &RailView) -> Result<Vec<String>> {
        self.draw("show", view)
    }

    /// Redraws the rail if it is up (new unread counts, a logo it lacked); otherwise nothing.
    pub fn refresh(&self, view: &RailView) -> Result<Vec<String>> {
        self.draw("refresh", view)
    }

    fn draw(&self, command: &str, view: &RailView) -> Result<Vec<String>> {
        let payload =
            serde_json::to_value(view).map_err(|error| Error::Plugin(error.to_string()))?;

        Ok(self.call::<Drawn>(command, payload)?.missing)
    }

    pub fn hide(&self) -> Result<()> {
        self.call::<Value>("hide", Value::Null).map(drop)
    }

    /// Where the user's choices go. Until this is called they are dropped (the rail still closes).
    pub fn on_event<F>(&self, handler: F) -> Result<()>
    where
        F: Fn(RailEvent) + Send + Sync + 'static,
    {
        #[cfg(target_os = "android")]
        {
            let channel = tauri::ipc::Channel::<Value>::new(move |message| {
                // anything malformed is dropped, never propagated
                if let Ok(event) = message.deserialize::<RailEvent>() {
                    handler(event);
                }

                Ok(())
            });

            self.call::<Value>("setEventHandler", json!({ "handler": channel }))
                .map(drop)
        }

        #[cfg(not(target_os = "android"))]
        {
            drop(handler);
            let _ = json!(null);

            Ok(())
        }
    }
}

pub trait RailExt<R: Runtime> {
    fn shiver_rail(&self) -> &Rail<R>;
}

impl<R: Runtime, T: Manager<R>> RailExt<R> for T {
    fn shiver_rail(&self) -> &Rail<R> {
        self.state::<Rail<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("shiver-rail")
        .setup(|app, _api| {
            #[cfg(target_os = "android")]
            app.manage(Rail {
                handle: _api.register_android_plugin("com.shiver.rail", "RailPlugin")?,
            });

            #[cfg(not(target_os = "android"))]
            app.manage(Rail::<R> {
                marker: std::marker::PhantomData,
            });

            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_are_read_from_what_the_rail_sends() {
        let read = |json: &str| serde_json::from_str::<RailEvent>(json).ok();

        assert_eq!(
            read(r#"{"kind":"open","entryId":"a"}"#),
            Some(RailEvent::Open {
                entry_id: "a".into()
            })
        );
        assert_eq!(read(r#"{"kind":"dms"}"#), Some(RailEvent::Dms));
        assert_eq!(read(r#"{"kind":"add"}"#), Some(RailEvent::Add));
        assert_eq!(read(r#"{"kind":"settings"}"#), Some(RailEvent::Settings));
        assert_eq!(read(r#"{"kind":"closed"}"#), Some(RailEvent::Closed));
        assert_eq!(
            read(r#"{"kind":"folder","folderId":"f","expanded":true}"#),
            Some(RailEvent::Folder {
                folder_id: "f".into(),
                expanded: true
            })
        );
        assert_eq!(read(r#"{"kind":"folder","folderId":"f"}"#), None);
        assert_eq!(read(r#"{"kind":"home"}"#), None);
        assert_eq!(read(r#"{"kind":"open"}"#), None);
        assert_eq!(read(r#"{"kind":"elsewhere"}"#), None);
    }

    #[test]
    fn the_view_is_what_the_page_reads() {
        let view = serde_json::to_value(RailView {
            theme_color: "#0a0a0a".into(),
            accent_color: "#e5e5e5".into(),
            text_color: None,
            current: Some("a".into()),
            servers: vec![Tile {
                id: "a".into(),
                name: "Chat".into(),
                folder_id: Some("f".into()),
                position: 0,
                unread: 2,
                icon_key: None,
                icon: None,
            }],
            folders: vec![RailFolder {
                id: "f".into(),
                name: "Work".into(),
                position: 0,
                expanded: false,
            }],
        })
        .unwrap();

        assert_eq!(view["themeColor"], "#0a0a0a");
        assert_eq!(view["textColor"], Value::Null);
        assert_eq!(view["servers"][0]["folderId"], "f");
        assert_eq!(view["servers"][0]["iconKey"], Value::Null);
        assert_eq!(view["folders"][0]["expanded"], false);
    }
}
