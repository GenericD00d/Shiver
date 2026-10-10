//! A server's or folder's menu, from the item model `shared/web/menus.ts` defines, shown as a native
//! menu; its ids and size are checked, and the choice comes back as `shiver://server-menu`.

use tauri::{
    menu::{
        CheckMenuItemBuilder, ContextMenu, IsMenuItem, MenuBuilder, MenuItemBuilder,
        PredefinedMenuItem, SubmenuBuilder,
    },
    AppHandle, Manager, Wry,
};

use crate::{
    error::{Core, Result},
    secrets::{self, Secret},
    webviews,
};

/// What a server's menu says beyond its entry (the menu itself is built by the shell, from the item
/// model both clients share).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuFacts {
    /// Shiver keeps its password, so there is one to forget
    has_password: bool,
    /// whether Shiver has connected and looked for the companion plugin yet
    plugin_checked: bool,
    /// the plugin's version, when it was found
    plugin: Option<String>,
    /// it has sent a message over the size limit this run
    too_large: bool,
}

#[tauri::command]
pub async fn server_menu_facts(app: AppHandle, id: String) -> MenuFacts {
    // a keychain read, off the UI thread
    let has_password = secrets::read_off_thread(Secret::Password, &id)
        .await
        .is_some();
    let plugin = app.state::<crate::watch::Plugins>().all().remove(&id);

    MenuFacts {
        has_password,
        plugin_checked: plugin.is_some(),
        plugin: plugin.flatten(),
        too_large: app.state::<crate::watch::Reported>().mentioned(&id),
    }
}

/// One entry of a rail menu, as `shared/web/menus.ts` builds it.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MenuEntry {
    Item {
        id: String,
        label: String,
        checked: Option<bool>,
        disabled: Option<bool>,
    },
    Separator,
    Submenu {
        label: String,
        items: Vec<MenuEntry>,
    },
}

/// More than any rail menu has; a menu past it is refused rather than drawn.
const MAX_MENU_ENTRIES: usize = 64;

const MAX_MENU_LABEL: usize = 120;

const MAX_MENU_ID: usize = 200;

/// An item id is `action:target`: letters and dashes, then ids (uuids) joined by colons. That is all
/// the shell's menus make, and all `lib.rs` hands back to it.
fn is_menu_id(id: &str) -> bool {
    let Some((action, target)) = id.split_once(':') else {
        return false;
    };

    id.len() <= MAX_MENU_ID
        && !action.is_empty()
        && action.chars().all(|c| c.is_ascii_lowercase() || c == '-')
        && !target.is_empty()
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':')
}

fn count_entries(entries: &[MenuEntry]) -> usize {
    entries
        .iter()
        .map(|entry| match entry {
            MenuEntry::Submenu { items, .. } => 1 + count_entries(items),
            _ => 1,
        })
        .sum()
}

fn menu_label(label: &str) -> String {
    shiver_core::text::clamp(label.to_string(), MAX_MENU_LABEL)
}

fn refuse_menu() -> crate::error::Error {
    Core::InvalidInput("That menu cannot be shown.".into()).into()
}

/// The native items for `entries`; a submenu may hold items and separators only.
fn menu_items(
    app: &AppHandle,
    entries: &[MenuEntry],
    nested: bool,
) -> Result<Vec<Box<dyn IsMenuItem<Wry>>>> {
    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::with_capacity(entries.len());

    for entry in entries {
        match entry {
            MenuEntry::Separator => items.push(Box::new(PredefinedMenuItem::separator(app)?)),
            MenuEntry::Item {
                id,
                label,
                checked,
                disabled,
            } => {
                if !is_menu_id(id) {
                    return Err(refuse_menu());
                }

                let enabled = !disabled.unwrap_or(false);

                match checked {
                    Some(checked) => items.push(Box::new(
                        CheckMenuItemBuilder::with_id(id.as_str(), menu_label(label))
                            .checked(*checked)
                            .enabled(enabled)
                            .build(app)?,
                    )),
                    None => items.push(Box::new(
                        MenuItemBuilder::with_id(id.as_str(), menu_label(label))
                            .enabled(enabled)
                            .build(app)?,
                    )),
                }
            }
            MenuEntry::Submenu { .. } if nested => return Err(refuse_menu()),
            MenuEntry::Submenu {
                label,
                items: inner,
            } => {
                let inner = menu_items(app, inner, true)?;
                let refs: Vec<&dyn IsMenuItem<Wry>> = inner.iter().map(AsRef::as_ref).collect();

                items.push(Box::new(
                    SubmenuBuilder::new(app, menu_label(label))
                        .items(&refs)
                        .build()?,
                ));
            }
        }
    }

    Ok(items)
}

/// Shows a rail menu as a native menu at the pointer (an html one would be painted over by the
/// server webview). The shell builds it from the item model both clients share; a chosen item's id
/// comes back to the shell through `lib.rs`.
#[tauri::command]
pub async fn show_menu(app: AppHandle, entries: Vec<MenuEntry>) -> Result<()> {
    if entries.is_empty() || count_entries(&entries) > MAX_MENU_ENTRIES {
        return Err(refuse_menu());
    }

    let window = webviews::main_window(&app)?;
    let items = menu_items(&app, &entries, false)?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(AsRef::as_ref).collect();

    MenuBuilder::new(&app).items(&refs).build()?.popup(window)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_menu_item_id_is_an_action_and_its_target() {
        assert!(is_menu_id("open:6f1c2e4a-0b7d-4c1e-9a55-3d2f8e1b7c90"));
        assert!(is_menu_id("move:server-id:folder-id"));
        assert!(!is_menu_id("quit"));
        assert!(!is_menu_id("open:"));
        assert!(!is_menu_id(":id"));
        assert!(!is_menu_id("Open:id"));
        assert!(!is_menu_id("open:id/../x"));
        assert!(!is_menu_id(&format!("open:{}", "a".repeat(MAX_MENU_ID))));
    }

    #[test]
    fn a_menu_counts_what_its_submenus_hold() {
        let entries: Vec<MenuEntry> = serde_json::from_str(
            r#"[{"kind":"item","id":"open:s","label":"Open"},{"kind":"separator"},
                {"kind":"submenu","label":"Notify","items":[{"kind":"item","id":"notify-all:s","label":"All","checked":true}]}]"#,
        )
        .expect("the shell's menu reads");

        assert_eq!(count_entries(&entries), 4);
    }
}
