//! The rail's structure: servers and folders sharing one top-level order, folder members keeping
//! their own. Generic over each client's server entry type.

use serde::Deserialize;

use crate::{model::Folder, Error, Result};

pub const MAX_FOLDER_NAME: usize = 100;

/// What the rail needs from a client's server entry.
pub trait RailServer {
    fn id(&self) -> &str;
    fn folder_id(&self) -> Option<&str>;
    fn set_folder_id(&mut self, folder_id: Option<String>);
    fn position(&self) -> i32;
    fn set_position(&mut self, position: i32);
}

/// One top-level rail item, as the frontends report their order.
#[derive(Debug, Deserialize)]
pub struct RailRef {
    /// `server` or `folder`
    pub kind: String,
    pub id: String,
}

/// Implements [`RailServer`] for a struct with `id`, `folder_id` and `position` fields.
#[macro_export]
macro_rules! rail_server {
    ($type:ty) => {
        impl $crate::rail::RailServer for $type {
            fn id(&self) -> &str {
                &self.id
            }

            fn folder_id(&self) -> Option<&str> {
                self.folder_id.as_deref()
            }

            fn set_folder_id(&mut self, folder_id: Option<String>) {
                self.folder_id = folder_id;
            }

            fn position(&self) -> i32 {
                self.position
            }

            fn set_position(&mut self, position: i32) {
                self.position = position;
            }
        }
    };
}

/// The next free top-level position (servers in folders keep their own numbering).
pub fn next_position<S: RailServer>(servers: &[S], folders: &[Folder]) -> i32 {
    servers
        .iter()
        .filter(|server| server.folder_id().is_none())
        .map(RailServer::position)
        .chain(folders.iter().map(|folder| folder.position))
        .max()
        .map_or(0, |max| max + 1)
}

/// A client's servers and folders, borrowed together.
pub struct Rail<'a, S> {
    pub servers: &'a mut Vec<S>,
    pub folders: &'a mut Vec<Folder>,
}

/// A trimmed, non-empty folder name of at most `MAX_FOLDER_NAME` characters.
pub fn folder_name(name: &str) -> Result<String> {
    let trimmed = name.trim();

    if trimmed.is_empty() {
        return Err(Error::InvalidInput("A folder needs a name".into()));
    }

    Ok(trimmed.chars().take(MAX_FOLDER_NAME).collect())
}

impl<S: RailServer> Rail<'_, S> {
    fn server_mut(&mut self, id: &str) -> Result<&mut S> {
        self.servers
            .iter_mut()
            .find(|server| server.id() == id)
            .ok_or(Error::UnknownServer)
    }

    fn folder_mut(&mut self, id: &str) -> Result<&mut Folder> {
        self.folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(Error::UnknownFolder)
    }

    pub fn next_position(&self) -> i32 {
        next_position(self.servers, self.folders)
    }

    /// A folder `id` holding `member_ids` (in that order), where its first member was.
    pub fn create_folder(
        &mut self,
        id: String,
        name: &str,
        member_ids: &[String],
    ) -> Result<Folder> {
        let position = self
            .servers
            .iter()
            .filter(|server| member_ids.iter().any(|member| member == server.id()))
            .map(RailServer::position)
            .min()
            .unwrap_or_else(|| self.next_position());

        let folder = Folder {
            id,
            name: folder_name(name)?,
            position,
            expanded: true,
        };

        for (index, member) in member_ids.iter().enumerate() {
            let server = self.server_mut(member)?;

            server.set_folder_id(Some(folder.id.clone()));
            server.set_position(index as i32);
        }

        self.folders.push(folder.clone());

        Ok(folder)
    }

    pub fn rename_folder(&mut self, id: &str, name: &str) -> Result<()> {
        self.folder_mut(id)?.name = folder_name(name)?;

        Ok(())
    }

    pub fn set_folder_expanded(&mut self, id: &str, expanded: bool) -> Result<()> {
        self.folder_mut(id)?.expanded = expanded;

        Ok(())
    }

    /// Removes a folder; its servers move to the top level.
    pub fn delete_folder(&mut self, id: &str) -> Result<()> {
        self.folder_mut(id)?;

        for server in self
            .servers
            .iter_mut()
            .filter(|server| server.folder_id() == Some(id))
        {
            server.set_folder_id(None);
        }

        self.folders.retain(|folder| folder.id != id);

        Ok(())
    }

    /// Moves a server into a folder, or out of any with `None`.
    pub fn set_server_folder(&mut self, id: &str, folder_id: Option<String>) -> Result<()> {
        if let Some(folder_id) = &folder_id {
            self.folder_mut(folder_id)?;
        }

        self.server_mut(id)?.set_folder_id(folder_id);

        Ok(())
    }

    /// Puts one top-level item at `position`.
    pub fn place(&mut self, item: &RailRef, position: i32) -> Result<()> {
        match item.kind.as_str() {
            "folder" => self.folder_mut(&item.id)?.position = position,
            "server" => self.server_mut(&item.id)?.set_position(position),
            other => {
                return Err(Error::InvalidInput(format!(
                    "'{other}' is not a kind of rail item"
                )))
            }
        }

        Ok(())
    }

    /// Numbers the top level in the given order.
    pub fn reorder(&mut self, ordered: &[RailRef]) -> Result<()> {
        for (index, item) in ordered.iter().enumerate() {
            self.place(item, index as i32)?;
        }

        Ok(())
    }

    /// Numbers servers in the given order (within whatever list each belongs to).
    pub fn reorder_servers(&mut self, ordered_ids: &[String]) -> Result<()> {
        for (index, id) in ordered_ids.iter().enumerate() {
            self.server_mut(id)?.set_position(index as i32);
        }

        self.servers.sort_by_key(RailServer::position);

        Ok(())
    }

    /// Dissolves folders holding fewer than two servers: their servers take the folder's place,
    /// and the top level is renumbered 0, 1, 2… Returns whether anything changed.
    pub fn prune_folders(&mut self) -> bool {
        let servers = &*self.servers;
        let doomed: Vec<(String, i32)> = self
            .folders
            .iter()
            .filter(|folder| {
                servers
                    .iter()
                    .filter(|server| server.folder_id() == Some(folder.id.as_str()))
                    .count()
                    < 2
            })
            .map(|folder| (folder.id.clone(), folder.position))
            .collect();

        if doomed.is_empty() {
            return false;
        }

        for server in self.servers.iter_mut() {
            let place = server
                .folder_id()
                .and_then(|id| doomed.iter().find(|(doomed, _)| doomed == id))
                .map(|(_, place)| *place);

            if let Some(place) = place {
                server.set_position(place);
                server.set_folder_id(None);
            }
        }

        self.folders
            .retain(|folder| !doomed.iter().any(|(id, _)| id == &folder.id));

        let mut items: Vec<(i32, bool, String)> = self
            .servers
            .iter()
            .filter(|server| server.folder_id().is_none())
            .map(|server| (server.position(), false, server.id().to_string()))
            .chain(
                self.folders
                    .iter()
                    .map(|folder| (folder.position, true, folder.id.clone())),
            )
            .collect();

        items.sort_by_key(|(position, _, _)| *position);

        for (index, (_, is_folder, id)) in items.iter().enumerate() {
            if *is_folder {
                if let Ok(folder) = self.folder_mut(id) {
                    folder.position = index as i32;
                }
            } else if let Ok(server) = self.server_mut(id) {
                server.set_position(index as i32);
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct Entry {
        id: String,
        folder_id: Option<String>,
        position: i32,
    }

    crate::rail_server!(Entry);

    fn at(id: &str, folder: Option<&str>, position: i32) -> Entry {
        Entry {
            id: id.into(),
            folder_id: folder.map(str::to_string),
            position,
        }
    }

    fn folder(id: &str, position: i32) -> Folder {
        Folder {
            id: id.into(),
            name: id.into(),
            position,
            expanded: true,
        }
    }

    #[test]
    fn a_folder_holding_fewer_than_two_servers_dissolves() {
        let mut servers = vec![
            at("a", Some("keep"), 0),
            at("b", Some("keep"), 1),
            at("c", Some("lonely"), 0),
            at("d", None, 3),
        ];
        let mut folders = vec![folder("keep", 0), folder("lonely", 1), folder("empty", 2)];
        let mut rail = Rail {
            servers: &mut servers,
            folders: &mut folders,
        };

        assert!(rail.prune_folders());
        assert_eq!(
            folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            vec!["keep"]
        );
        assert_eq!(
            servers.iter().find(|s| s.id == "c").unwrap().folder_id,
            None
        );
    }

    #[test]
    fn pruning_a_tidy_rail_changes_nothing() {
        let mut servers = vec![at("a", Some("keep"), 0), at("b", Some("keep"), 1)];
        let mut folders = vec![folder("keep", 0)];

        assert!(!Rail {
            servers: &mut servers,
            folders: &mut folders
        }
        .prune_folders());
    }

    #[test]
    fn a_freed_server_lands_where_its_folder_was() {
        let mut servers = vec![
            at("first", None, 0),
            at("freed", Some("lonely"), 0),
            at("last", None, 2),
        ];
        let mut folders = vec![folder("lonely", 1)];

        assert!(Rail {
            servers: &mut servers,
            folders: &mut folders
        }
        .prune_folders());

        servers.sort_by_key(|server| server.position);

        let order: Vec<(&str, i32)> = servers
            .iter()
            .map(|server| (server.id.as_str(), server.position))
            .collect();

        assert_eq!(order, vec![("first", 0), ("freed", 1), ("last", 2)]);
    }

    #[test]
    fn folder_members_do_not_count_towards_the_next_position() {
        let mut servers = vec![at("a", None, 0), at("b", Some("f"), 7)];
        let mut folders = vec![folder("f", 1)];

        assert_eq!(
            Rail {
                servers: &mut servers,
                folders: &mut folders
            }
            .next_position(),
            2
        );
    }

    #[test]
    fn a_new_folder_takes_its_first_members_place() {
        let mut servers = vec![at("a", None, 0), at("b", None, 1), at("c", None, 2)];
        let mut folders = vec![];
        let mut rail = Rail {
            servers: &mut servers,
            folders: &mut folders,
        };
        let made = rail
            .create_folder("f".into(), "  Games  ", &["c".into(), "b".into()])
            .unwrap();

        assert_eq!((made.name.as_str(), made.position), ("Games", 1));
        assert_eq!(servers[2].position, 0);
        assert_eq!(servers[1].position, 1);
        assert!(folder_name("   ").is_err());
        assert_eq!(
            folder_name(&"x".repeat(500)).unwrap().len(),
            MAX_FOLDER_NAME
        );
    }

    #[test]
    fn unknown_ids_and_kinds_are_refused() {
        let mut servers = vec![at("a", None, 0)];
        let mut folders = vec![];
        let mut rail = Rail {
            servers: &mut servers,
            folders: &mut folders,
        };

        assert!(matches!(
            rail.set_server_folder("a", Some("nope".into())),
            Err(Error::UnknownFolder)
        ));
        assert!(matches!(
            rail.reorder(&[RailRef {
                kind: "server".into(),
                id: "zz".into()
            }]),
            Err(Error::UnknownServer)
        ));
        assert!(matches!(
            rail.reorder(&[RailRef {
                kind: "tile".into(),
                id: "a".into()
            }]),
            Err(Error::InvalidInput(_))
        ));
    }
}
