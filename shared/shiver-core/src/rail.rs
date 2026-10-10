//! The rail's structure: servers and folders sharing one top-level order, folder members keeping
//! their own. Generic over each client's server entry type.

use serde::Deserialize;

use crate::{model::Folder, Error, Result};

const MAX_FOLDER_NAME: usize = 100;

/// What the rail needs from a client's server entry.
pub trait RailServer {
    fn id(&self) -> &str;
    fn folder_id(&self) -> Option<&str>;
    fn set_folder_id(&mut self, folder_id: Option<String>);
    fn position(&self) -> i32;
    fn set_position(&mut self, position: i32);
}

/// What a top-level rail item is. Anything else is refused where the frontend's order is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RailKind {
    Server,
    Folder,
}

/// One top-level rail item, as the frontends report their order.
#[derive(Debug, Clone, Deserialize)]
pub struct RailRef {
    pub kind: RailKind,
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

/// [`rail_server!`] for a client's server entry, plus the lookups both clients' registries share
/// (a registry with `servers`, `folders` and `muted` fields).
#[macro_export]
macro_rules! registry {
    ($registry:ty, $server:ty) => {
        $crate::rail_server!($server);

        impl $registry {
            pub fn server(&self, id: &str) -> Option<&$server> {
                self.servers.iter().find(|server| server.id == id)
            }

            pub fn server_mut(&mut self, id: &str) -> Option<&mut $server> {
                self.servers.iter_mut().find(|server| server.id == id)
            }

            pub fn muted_for(&self, entry_id: &str) -> Vec<i64> {
                $crate::model::muted_for(&self.muted, entry_id)
            }

            /// Replaces one entry's mutes; returns the new list.
            pub fn set_muted_for(
                &mut self,
                entry_id: &str,
                channels: impl IntoIterator<Item = i64>,
            ) -> Vec<i64> {
                $crate::model::set_muted_for(&mut self.muted, entry_id, channels)
            }

            /// The next free top-level position.
            pub fn next_position(&self) -> i32 {
                $crate::rail::next_position(&self.servers, &self.folders)
            }

            pub fn rail(&mut self) -> $crate::rail::Rail<'_, $server> {
                $crate::rail::Rail {
                    servers: &mut self.servers,
                    folders: &mut self.folders,
                }
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
fn folder_name(name: &str) -> Result<String> {
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

    /// A folder `id` holding `member_ids` (in that order), where its first member was (a member
    /// taken from another folder was where that folder is). A folder a member left that is now too
    /// thin is dissolved.
    pub fn create_folder(
        &mut self,
        id: String,
        name: &str,
        member_ids: &[String],
    ) -> Result<Folder> {
        let slot = |server: &S| {
            server
                .folder_id()
                .and_then(|folder| self.folders.iter().find(|candidate| candidate.id == folder))
                .map_or(server.position(), |folder| folder.position)
        };
        let position = self
            .servers
            .iter()
            .filter(|server| member_ids.iter().any(|member| member == server.id()))
            .map(slot)
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

        let id = folder.id.clone();

        self.folders.push(folder);
        // the folder a member came from may stay, on the same place: the new one goes first
        self.renumber(|kind, item| i32::from(!(kind == RailKind::Folder && item == id)));

        let made = self.folder_mut(&id)?.clone();

        self.prune_folders();

        Ok(made)
    }

    pub fn rename_folder(&mut self, id: &str, name: &str) -> Result<()> {
        self.folder_mut(id)?.name = folder_name(name)?;

        Ok(())
    }

    pub fn set_folder_expanded(&mut self, id: &str, expanded: bool) -> Result<()> {
        self.folder_mut(id)?.expanded = expanded;

        Ok(())
    }

    /// Removes a folder; its servers move to the top level, in their order, where it was.
    pub fn delete_folder(&mut self, id: &str) -> Result<()> {
        let place = self.folder_mut(id)?.position;
        let mut freed: Vec<(String, i32)> = Vec::new();

        for server in self
            .servers
            .iter_mut()
            .filter(|server| server.folder_id() == Some(id))
        {
            freed.push((server.id().to_string(), server.position()));
            server.set_folder_id(None);
            server.set_position(place);
        }

        self.folders.retain(|folder| folder.id != id);
        self.renumber(|kind, item| {
            freed
                .iter()
                .find(|(freed, _)| kind == RailKind::Server && freed == item)
                .map_or(0, |(_, within)| within.saturating_add(1))
        });

        Ok(())
    }

    /// Moves a server into a folder (last in it), or out of any with `None` (just after the folder
    /// it left). A folder it left that is now too thin is dissolved.
    pub fn set_server_folder(&mut self, id: &str, folder_id: Option<String>) -> Result<()> {
        let position = match &folder_id {
            Some(folder_id) => {
                self.folder_mut(folder_id)?;

                self.servers
                    .iter()
                    .filter(|server| server.folder_id() == Some(folder_id) && server.id() != id)
                    .map(RailServer::position)
                    .max()
                    .map_or(0, |last| last + 1)
            }
            None => {
                let server = self.server_mut(id)?;
                let left = server.folder_id().map(str::to_string);
                let own = server.position();

                left.and_then(|left| self.folders.iter().find(|folder| folder.id == left))
                    .map_or(own, |folder| folder.position)
            }
        };
        let leaving = folder_id.is_none();
        let server = self.server_mut(id)?;

        server.set_folder_id(folder_id);
        server.set_position(position);
        self.renumber(|kind, item| i32::from(leaving && kind == RailKind::Server && item == id));
        self.prune_folders();

        Ok(())
    }

    /// Puts one top-level item at `position`.
    pub fn place(&mut self, item: &RailRef, position: i32) -> Result<()> {
        match item.kind {
            RailKind::Folder => self.folder_mut(&item.id)?.position = position,
            RailKind::Server => self.server_mut(&item.id)?.set_position(position),
        }

        Ok(())
    }

    /// Numbers the top level in the given order. Items the order leaves out (it came from a view
    /// older than the registry) follow, in the order they had, rather than sharing a number.
    pub fn reorder(&mut self, ordered: &[RailRef]) -> Result<()> {
        let listed = |kind: RailKind, id: &str| {
            ordered
                .iter()
                .any(|item| item.kind == kind && item.id == id)
        };
        let mut rest: Vec<(i32, RailRef)> = self
            .servers
            .iter()
            .filter(|server| server.folder_id().is_none() && !listed(RailKind::Server, server.id()))
            .map(|server| (server.position(), RailKind::Server, server.id().to_string()))
            .chain(
                self.folders
                    .iter()
                    .filter(|folder| !listed(RailKind::Folder, &folder.id))
                    .map(|folder| (folder.position, RailKind::Folder, folder.id.clone())),
            )
            .map(|(position, kind, id)| (position, RailRef { kind, id }))
            .collect();

        rest.sort_by_key(|(position, _)| *position);

        for (index, item) in ordered
            .iter()
            .chain(rest.iter().map(|(_, item)| item))
            .enumerate()
        {
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
        self.renumber(|_, _| 0);

        true
    }

    /// Numbers the top level 0, 1, 2… by position, `tiebreak` ordering items that share one
    /// (lowest first; servers before folders after that).
    fn renumber(&mut self, tiebreak: impl Fn(RailKind, &str) -> i32) {
        let mut items: Vec<((i32, i32), RailRef)> = self
            .servers
            .iter()
            .filter(|server| server.folder_id().is_none())
            .map(|server| (server.position(), RailKind::Server, server.id().to_string()))
            .chain(
                self.folders
                    .iter()
                    .map(|folder| (folder.position, RailKind::Folder, folder.id.clone())),
            )
            .map(|(position, kind, id)| ((position, tiebreak(kind, &id)), RailRef { kind, id }))
            .collect();

        items.sort_by_key(|(key, _)| *key);

        for (index, (_, item)) in items.iter().enumerate() {
            let _ = self.place(item, index as i32);
        }
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
    fn a_folder_made_from_another_folders_member_takes_that_folders_place() {
        let mut servers = vec![
            at("a", None, 0),
            at("b", Some("old"), 0),
            at("c", Some("old"), 1),
            at("d", None, 2),
        ];
        let mut folders = vec![folder("old", 1)];
        let mut rail = Rail {
            servers: &mut servers,
            folders: &mut folders,
        };

        let made = rail
            .create_folder("new".into(), "New", &["d".into(), "c".into()])
            .unwrap();

        assert_eq!(made.position, 1);
        assert!(
            !folders.iter().any(|folder| folder.id == "old"),
            "left too thin"
        );
    }

    /// The top level as `(id, position)`, in order.
    fn top(servers: &[Entry], folders: &[Folder]) -> Vec<(String, i32)> {
        let mut items: Vec<(String, i32)> = servers
            .iter()
            .filter(|server| server.folder_id.is_none())
            .map(|server| (server.id.clone(), server.position))
            .chain(
                folders
                    .iter()
                    .map(|folder| (folder.id.clone(), folder.position)),
            )
            .collect();

        items.sort_by_key(|(_, position)| *position);
        items
    }

    fn named(items: &[(&str, i32)]) -> Vec<(String, i32)> {
        items.iter().map(|(id, at)| (id.to_string(), *at)).collect()
    }

    #[test]
    fn a_deleted_folders_servers_take_its_place_in_their_order() {
        let mut servers = vec![
            at("a", None, 0),
            at("y", Some("mid"), 1),
            at("x", Some("mid"), 0),
            at("z", None, 2),
        ];
        let mut folders = vec![folder("mid", 1)];

        Rail {
            servers: &mut servers,
            folders: &mut folders,
        }
        .delete_folder("mid")
        .unwrap();

        assert_eq!(
            top(&servers, &folders),
            named(&[("a", 0), ("x", 1), ("y", 2), ("z", 3)])
        );
    }

    #[test]
    fn a_server_moved_out_of_a_folder_that_stays_lands_after_it() {
        let mut servers = vec![
            at("a", None, 0),
            at("x", Some("f"), 0),
            at("y", Some("f"), 1),
            at("w", Some("f"), 2),
            at("z", None, 2),
        ];
        let mut folders = vec![folder("f", 1)];
        let mut rail = Rail {
            servers: &mut servers,
            folders: &mut folders,
        };

        rail.set_server_folder("x", None).unwrap();
        assert!(!rail.prune_folders());
        assert_eq!(
            top(&servers, &folders),
            named(&[("a", 0), ("f", 1), ("x", 2), ("z", 3)])
        );
    }

    #[test]
    fn a_server_moved_into_a_folder_goes_last_in_it() {
        let mut servers = vec![
            at("x", Some("f"), 0),
            at("y", Some("f"), 1),
            at("a", None, 1),
        ];
        let mut folders = vec![folder("f", 0)];

        Rail {
            servers: &mut servers,
            folders: &mut folders,
        }
        .set_server_folder("a", Some("f".into()))
        .unwrap();

        assert_eq!(servers[2].position, 2);
        assert_eq!(top(&servers, &folders), named(&[("f", 0)]));
    }

    #[test]
    fn a_folder_made_from_a_folder_that_stays_does_not_share_its_place() {
        let mut servers = vec![
            at("a", None, 0),
            at("x", Some("old"), 0),
            at("y", Some("old"), 1),
            at("w", Some("old"), 2),
            at("d", None, 2),
        ];
        let mut folders = vec![folder("old", 1)];
        let mut rail = Rail {
            servers: &mut servers,
            folders: &mut folders,
        };

        let made = rail
            .create_folder("new".into(), "New", &["w".into(), "d".into()])
            .unwrap();

        assert!(!rail.prune_folders());
        assert_eq!(made.position, 1);
        assert_eq!(
            top(&servers, &folders),
            named(&[("a", 0), ("new", 1), ("old", 2)])
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
                kind: RailKind::Server,
                id: "zz".into()
            }]),
            Err(Error::UnknownServer)
        ));
        assert!(serde_json::from_str::<RailRef>(r#"{"kind":"tile","id":"a"}"#).is_err());
        assert_eq!(
            serde_json::from_str::<RailRef>(r#"{"kind":"folder","id":"f"}"#)
                .unwrap()
                .kind,
            RailKind::Folder
        );
    }

    #[test]
    fn items_an_order_leaves_out_follow_it_in_their_old_order() {
        let mut servers = vec![
            at("a", None, 0),
            at("b", None, 1),
            at("c", None, 2),
            at("in", Some("f"), 0),
        ];
        let mut folders = vec![folder("f", 3)];
        let mut rail = Rail {
            servers: &mut servers,
            folders: &mut folders,
        };
        let server = |id: &str| RailRef {
            kind: RailKind::Server,
            id: id.into(),
        };

        rail.reorder(&[server("c"), server("a")]).unwrap();

        let position = |id: &str| servers.iter().find(|s| s.id == id).unwrap().position;

        assert_eq!(
            (
                position("c"),
                position("a"),
                position("b"),
                folders[0].position
            ),
            (0, 1, 2, 3)
        );
        assert_eq!(position("in"), 0, "a folder's servers keep their own order");
    }
}
