import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import type { ConfirmAction } from './Boot';
import { MessagesIcon, PlusIcon, SettingsIcon } from './icons';
import type { Folder, ServerEntry } from '../types';
import { byPosition, initials, membersOf } from '../../../shared/web/rail';

type Props = {
  servers: ServerEntry[];
  /** the server whose tile is marked, or null on Shiver's own screens */
  activeId: string | null;
  /** which of Shiver's own screens is up, so its tile can be marked the same way */
  screen: 'boot' | 'add' | 'settings' | 'signIn' | 'dms';
  /** unread per server id; a server missing from it has none */
  unread: Record<string, number>;
  onOpen: (id: string) => void;
  onOpenDms: () => void;
  onAdd: () => void;
  onSettings: () => void;
  onRefresh: (id: string) => void;
  /** an action to confirm before it runs */
  onAsk: (action: ConfirmAction, id: string) => void;
  /** the rail's folders, ordered among the loose servers by `position` */
  folders: Folder[];
  /** the rail's top level after a drag, folders and loose servers together */
  onReorder: (ordered: RailRef[]) => void;
  /** moves a server into a folder, or out of one when given null */
  onSetFolder: (serverId: string, folderId: string | null) => void;
  /** makes a folder holding these servers */
  onCreateFolder: (memberIds: string[]) => void;
  onRenameFolder: (id: string, name: string) => void;
  onDeleteFolder: (id: string) => void;
  onToggleFolder: (id: string, expanded: boolean) => void;
};

/** One row of the rail's top level. */
export type RailRef = { kind: 'server' | 'folder'; id: string };

/** Matches the desktop rail, which is the other place Shiver counts unread. */
const badgeLabel = (count: number) => (count > 99 ? '99+' : String(count));

/** How long a press has to be held to mean "menu" rather than "open". */
const HOLD_MS = 500;
/** How far a finger may drift during that hold and still be a press rather than a scroll. */
const HOLD_SLOP = 10;
/** How long after a press ends its own click may still arrive and need ignoring. */
const CLICK_GRACE_MS = 700;

/** A long press (its own timer: `contextmenu` is unreliable on touch and brings the selection menu). */
const useLongPress = (
  onHold: (id: string, at: number) => void,
  drag: {
    /** the finger has moved to this point while a tile is lifted */
    over: (id: string, y: number) => void;
    /** keep the lifted tile under the finger, or put it back when given nothing */
    carry: (id: string | null, y: number, instant?: boolean) => void;
    /** the drag ended having actually moved the tile, which is named here */
    drop: (dragged: string) => void;
  }
) => {
  const timer = useRef(0);
  const from = useRef({ x: 0, y: 0 });
  /** When a press last ended in a menu or drag, so its click is ignored (a time, never a stale flag). */
  const consumedAt = useRef(0);
  const lifted = useRef<string | null>(null);
  const moved = useRef(false);

  const cancel = useCallback(() => {
    window.clearTimeout(timer.current);
    timer.current = 0;
  }, []);

  useEffect(() => cancel, [cancel]);

  const end = useCallback(() => {
    cancel();

    if (!lifted.current) return;

    const id = lifted.current;
    const at = from.current.y;
    const wasDrag = moved.current;

    lifted.current = null;
    moved.current = false;
    consumedAt.current = Date.now();

    drag.carry(null, 0);

    // held still it was always the menu; moved, it was a drag and the order is what matters
    if (wasDrag) drag.drop(id);
    else onHold(id, at);
  }, [cancel, drag, onHold]);

  return {
    /** true when the press that just ended had already opened a menu or moved a tile */
    consumed: () => {
      const was = Date.now() - consumedAt.current < CLICK_GRACE_MS;

      consumedAt.current = 0;

      return was;
    },
    /** the tile currently picked up, so it can be drawn as lifted */
    lifted: () => lifted.current,
    handlers: (id: string) => ({
      onTouchStart: (event: React.TouchEvent) => {
        const touch = event.touches[0];

        if (!touch) return;

        from.current = { x: touch.clientX, y: touch.clientY };
        moved.current = false;

        timer.current = window.setTimeout(() => {
          // Lifted, not yet decided. Held still this is the menu, as it always was; moved, it is a
          // drag. Only what happens next tells them apart, so neither is committed to here.
          lifted.current = id;

          // and it goes to the finger, which is the rail saying it has been picked up
          drag.carry(id, from.current.y);
        }, HOLD_MS);
      },
      onTouchMove: (event: React.TouchEvent) => {
        const touch = event.touches[0];

        if (!touch) return;

        const drifted =
          Math.abs(touch.clientX - from.current.x) > HOLD_SLOP ||
          Math.abs(touch.clientY - from.current.y) > HOLD_SLOP;

        // before the hold lands, a finger that wanders is a scroll rather than a press
        if (!lifted.current) {
          if (drifted) cancel();

          return;
        }

        if (drifted) moved.current = true;

        // instant from here: the lift travels on the class's easing, the carrying must not trail
        drag.carry(lifted.current, touch.clientY, true);
        drag.over(lifted.current, touch.clientY);
      },
      onTouchEnd: end,
      onTouchCancel: end,
      onContextMenu: (event: React.MouseEvent) => event.preventDefault()
    })
  };

};

/** A server's logo: the copy Shiver inlined (no request to the server), else its address. */
export const iconOf = (server: ServerEntry) => server.iconData ?? server.iconUrl ?? undefined;

/** The server rail, on Shiver's own page (a server's page is shown nothing of it). */
export const Rail = ({
  servers,
  activeId,
  screen,
  unread,
  onOpen,
  onOpenDms,
  onAdd,
  onSettings,
  onRefresh,
  onAsk,
  folders,
  onReorder,
  onSetFolder,
  onCreateFolder,
  onRenameFolder,
  onDeleteFolder,
  onToggleFolder
}: Props) => {
  const [menu, setMenu] = useState<{
    kind: 'server' | 'folder';
    id: string;
    at: number;
    /** the opening press's synthesised click lands on the scrim, which ignores it for a moment */
    openedAt: number;
  } | null>(null);
  /** the folder being renamed, and the text so far */
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  /** the tile a dragged one is resting on, which means the two would be grouped */
  const [onto, setOntoState] = useState<string | null>(null);
  /** the same, readable from inside the drag's callbacks, which close over their first render */
  const ontoRef = useRef<string | null>(null);

  /** how far the held tile has been pushed from where the layout put it */
  const carriedBy = useRef(0);
  /** the box each open folder draws its servers in, so a drag can tell when one has left */
  const groups = useRef(new Map<string, HTMLDivElement>());
  /** the folder a drag was pulled out of, applied on drop so the rail does not redraw mid-gesture */
  const pulledOut = useRef<string | null>(null);

  /**
   * Keeps the held tile under the finger, measured from its layout position (so reorders do not
   * drift) and written to the node directly (a render would lag a frame). Only the lift animates.
   */
  const carry = useCallback((key: string | null, y: number, instant = false) => {
    if (!key) {
      for (const node of tiles.current.values()) {
        node.style.transform = '';
        node.style.transition = '';
        node.classList.remove('lifted');
      }

      carriedBy.current = 0;

      return;
    }

    const node = tiles.current.get(key);

    if (!node) return;

    // Marked here as well as in the render, because a lift changes no state and so draws nothing:
    // the tile would move under the finger with none of the shadow, scale or standing-above-its
    // neighbours that say it has been picked up, until some later reorder happened to redraw it.
    // The next render works it out the same way and agrees.
    node.classList.add('lifted');

    const box = node.getBoundingClientRect();
    const home = box.top + box.height / 2 - carriedBy.current;

    carriedBy.current = y - home;

    // the scale belongs to the lifted class, and an inline transform replaces it whole
    node.style.transform = `translateY(${carriedBy.current}px) scale(1.12)`;

    if (instant) node.style.transition = 'none';
  }, []);

  const setOnto = (key: string | null) => {
    ontoRef.current = key;
    setOntoState(key);
  };
  /** the order on screen, ahead of the stored one during a drag */
  const [order, setOrder] = useState<string[]>([]);
  const tiles = useRef(new Map<string, HTMLButtonElement>());

  /** folders and loose servers in stored order (folder members keep their own order) */
  const top = useMemo(
    () =>
      byPosition([
        ...folders.map((folder) => ({ kind: 'folder' as const, id: folder.id, position: folder.position })),
        ...servers
          .filter((server) => !server.folderId)
          .map((server) => ({ kind: 'server' as const, id: server.id, position: server.position }))
      ]),
    [folders, servers]
  );

  const topIds = top.map((item) => `${item.kind}:${item.id}`).join();

  useEffect(() => {
    setOrder(top.map((item) => `${item.kind}:${item.id}`));
    // reset on the stored order rather than on every render, so a drag in progress is left alone
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [topIds]);

  const press = useLongPress(
    (key, at) => {
      const [kind, id] = key.split(':');

      setMenu({ kind: kind === 'folder' ? 'folder' : 'server', id, at, openedAt: Date.now() });
    },
    {
      over: (id, y) => {
        // Dragged clear of the group it was in, which is how a server leaves a folder. An open
        // folder's own tile is a slim bar, so "just above the folder" is a few pixels of travel and
        // no gesture at all; leaving the box the members are drawn in is what a finger pulling one
        // out is already doing.
        const from = [...groups.current.entries()].find(([, node]) =>
          [...node.children].some((child) => child === tiles.current.get(id))
        );

        if (from) {
          const inside = from[1].getBoundingClientRect();

          pulledOut.current = y < inside.top || y > inside.bottom ? from[0] : null;
        }

        const hit = [...tiles.current.entries()].find(([other, node]) => {
          if (other === id) return false;

          const box = node.getBoundingClientRect();

          return y >= box.top && y <= box.bottom;
        });

        if (!hit) return;

        const [overKey, node] = hit;
        const box = node.getBoundingClientRect();
        const offset = y - box.top;

        // The middle of a tile is not a place in the order, it is the tile itself: resting there
        // means these two belong together. The edges still mean before and after, and the band is
        // wider to leave than to enter so a wandering finger does not undo the decision.
        const grouping =
          ontoRef.current === overKey
            ? offset > box.height * 0.1 && offset < box.height * 0.9
            : offset > box.height * 0.25 && offset < box.height * 0.75;

        if (!id.startsWith('folder:') && grouping) {
          setOnto(overKey);

          return;
        }

        setOnto(null);

        // Above the tile's midpoint it goes before it, below it after — never simply "take its
        // place". Swapping the moment a tile is touched at all moves it out from under the finger,
        // which is both a jitter across the boundary and the reason the middle was unreachable:
        // one pixel into a tile and the dragged one already occupied it.
        const before = y < box.top + box.height / 2;

        setOrder((current) => {
          const from = current.indexOf(id);

          if (from < 0 || !current.includes(overKey)) return current;

          const next = [...current];
          const [moving] = next.splice(from, 1);
          const to = next.indexOf(overKey) + (before ? 0 : 1);

          next.splice(to, 0, moving);

          return next.every((key, index) => key === current[index]) ? current : next;
        });
      },
      carry,
      drop: (dragged) => {
        const target = ontoRef.current;
        const left = pulledOut.current;

        setOnto(null);

        pulledOut.current = null;

        // out of the group, and back to the top level — where the folder it left may now hold too
        // few servers to be one, which Shiver settles for itself
        if (!target && left && dragged.startsWith('server:')) {
          onSetFolder(dragged.slice('server:'.length), null);

          return;
        }

        // dropped on another tile: into that folder, or a folder made of the pair
        if (target && target !== dragged && dragged.startsWith('server:')) {
          const serverId = dragged.slice('server:'.length);
          const [kind, id] = target.split(':');

          if (kind === 'folder') onSetFolder(serverId, id);
          else onCreateFolder([id, serverId]);

          return;
        }

        setOrder((current) => {
          onReorder(
            current.map((key) => {
              const [kind, id] = key.split(':');

              return { kind: kind === 'folder' ? 'folder' : 'server', id } as RailRef;
            })
          );

          return current;
        });
      }
    }
  );

  const held = menu?.kind === 'server' ? servers.find((server) => server.id === menu.id) : undefined;
  const heldFolder =
    menu?.kind === 'folder' ? folders.find((folder) => folder.id === menu.id) : undefined;

  /** Closes the menu, unless the click is the one belonging to the press that opened it. */
  const dismissMenu = () => {
    if (menu && Date.now() - menu.openedAt < CLICK_GRACE_MS) return false;

    setMenu(null);

    return true;
  };


  const ordered = order
    .map((key) => {
      const [kind, id] = key.split(':');

      if (kind === 'folder') {
        const folder = folders.find((candidate) => candidate.id === id);

        return folder ? ({ kind: 'folder', folder } as const) : null;
      }

      const server = servers.find((candidate) => candidate.id === id);

      return server ? ({ kind: 'server', server } as const) : null;
    })
    .filter((item): item is NonNullable<typeof item> => !!item);

  /** A server tile, drawn the same whether it sits at the top level or inside a folder. */
  const serverTile = (server: ServerEntry, inFolder: boolean) => {
    const count = unread[server.id] ?? 0;
    const key = `server:${server.id}`;

    return (
      <button
        key={server.id}
        ref={(node) => {
          if (node) tiles.current.set(key, node);
          else tiles.current.delete(key);
        }}
        type="button"
        className={`rail-item${server.id === activeId ? ' active' : ''}${
          press.lifted() === key ? ' lifted' : ''
        }${inFolder ? ' in-folder' : ''}${onto === key ? ' drop-onto' : ''}`}
        title={count > 0 ? `${server.name} — ${count} unread` : server.name}
        onClick={() => {
          // the press that opened a menu, or moved a tile, is not also a tap on it
          if (press.consumed()) return;

          onOpen(server.id);
        }}
        {...press.handlers(key)}
      >
        {iconOf(server) ? (
          // not draggable: the platform's own image drag would cancel the touch Shiver is using
          <img src={iconOf(server)} alt="" draggable={false} />
        ) : (
          initials(server.name)
        )}

        {count > 0 ? <span className="rail-badge">{badgeLabel(count)}</span> : null}
      </button>
    );
  };

  return (
    <nav className="rail" aria-label="Servers">
      <button
        type="button"
        className={`rail-item${screen === 'dms' ? ' active' : ''}`}
        title="Direct messages"
        onClick={onOpenDms}
      >
        <MessagesIcon />
      </button>

      <div className="rail-divider" />

      {ordered.map((item) => {
        if (item.kind === 'server') return serverTile(item.server, false);

        const folder = item.folder;
        const members = membersOf(servers, folder.id);
        const key = `folder:${folder.id}`;

        return (
          <Fragment key={folder.id}>
            <button
              ref={(node) => {
                if (node) tiles.current.set(key, node);
                else tiles.current.delete(key);
              }}
              type="button"
              className={`rail-item rail-folder${folder.expanded ? ' open' : ''}${
                press.lifted() === key ? ' lifted' : ''
              }${onto === key ? ' drop-onto' : ''}`}
              title={folder.name}
              onClick={() => {
                if (press.consumed()) return;

                onToggleFolder(folder.id, !folder.expanded);
              }}
              {...press.handlers(key)}
            >
              {/* Open, the tile steps aside: the servers it holds are drawn underneath, and showing
                  them twice says nothing. Shut, the tile is the preview. */}
              {folder.expanded ? (
                <span className="folder-open" aria-hidden="true" />
              ) : (
                <span className="folder-grid">
                  {members.slice(0, 4).map((member) => (
                    <span className="folder-cell" key={member.id}>
                      {iconOf(member) ? (
                        <img src={iconOf(member)} alt="" draggable={false} />
                      ) : (
                        initials(member.name).slice(0, 1)
                      )}
                    </span>
                  ))}
                </span>
              )}
            </button>

            {folder.expanded ? (
              <div
                className="folder-contents"
                ref={(node) => {
                  if (node) groups.current.set(folder.id, node);
                  else groups.current.delete(folder.id);
                }}
              >
                {members.map((member) => serverTile(member, true))}
              </div>
            ) : null}
          </Fragment>
        );
      })}

      <button
        type="button"
        className={`rail-item rail-add${screen === 'add' ? ' active' : ''}`}
        title="Add a server"
        onClick={onAdd}
      >
        <PlusIcon />
      </button>

      <div className="rail-spacer" />

      <div className="rail-divider" />

      <button
        type="button"
        className={`rail-item${screen === 'settings' ? ' active' : ''}`}
        title="Shiver settings"
        onClick={onSettings}
      >
        <SettingsIcon />
      </button>

      {/* Marking every channel read needs that server's own client; its channel menu offers it. */}
      {held && menu ? (
        <>
          <div className="menu-scrim" onClick={() => dismissMenu()} />

          <div className="rail-menu" style={{ top: Math.max(8, menu.at - 24) }} role="menu">
            <button
              type="button"
              className="menu-item"
              onClick={() => {
                setMenu(null);
                onOpen(held.id);
              }}
            >
              Open
            </button>

            <button
              type="button"
              className="menu-item"
              onClick={() => {
                setMenu(null);
                onRefresh(held.id);
              }}
            >
              Refresh name and icon
            </button>

            <div className="menu-divider" />

            {/* Membership is a menu rather than a drop target. On a phone the tiles are 48px
                apart, and asking someone to land a finger inside one to mean "into this folder"
                rather than "next to it" is a worse gesture than simply saying which folder. */}
            {folders
              .filter((folder) => folder.id !== held.folderId)
              .map((folder) => (
                <button
                  key={folder.id}
                  type="button"
                  className="menu-item"
                  onClick={() => {
                    setMenu(null);
                    onSetFolder(held.id, folder.id);
                  }}
                >
                  Move to &ldquo;{folder.name}&rdquo;
                </button>
              ))}

            {held.folderId ? (
              <button
                type="button"
                className="menu-item"
                onClick={() => {
                  setMenu(null);
                  onSetFolder(held.id, null);
                }}
              >
                Take out of folder
              </button>
            ) : null}
            {/* No "new folder" here: a folder is made by dropping one server onto another, which is
                how the desktop rail does it, and it means a folder never exists without a pair to
                justify it. */}

            <div className="menu-divider" />

            {(
              [
                ['logout', 'Log out'],
                ['forgetpw', 'Forget my password'],
                ['remove', 'Remove from Shiver']
              ] as const
            ).map(([action, label]) => (
              <button
                key={action}
                type="button"
                className="menu-item"
                onClick={() => {
                  setMenu(null);
                  onAsk(action, held.id);
                }}
              >
                {label}
              </button>
            ))}
          </div>
        </>
      ) : null}

      {heldFolder && menu ? (
        <>
          <div
            className="menu-scrim"
            onClick={() => {
              if (!dismissMenu()) return;

              setRenaming(null);
            }}
          />

          <div className="rail-menu" style={{ top: Math.max(8, menu.at - 24) }} role="menu">
            {renaming?.id === heldFolder.id ? (
              <form
                className="menu-rename"
                onSubmit={(event) => {
                  event.preventDefault();

                  const name = renaming.name.trim();

                  setMenu(null);
                  setRenaming(null);

                  if (name) onRenameFolder(heldFolder.id, name);
                }}
              >
                <input
                  autoFocus
                  value={renaming.name}
                  aria-label="Folder name"
                  onChange={(event) => setRenaming({ id: heldFolder.id, name: event.target.value })}
                />
                <button type="submit" className="menu-item">
                  Save
                </button>
              </form>
            ) : (
              <button
                type="button"
                className="menu-item"
                onClick={() => setRenaming({ id: heldFolder.id, name: heldFolder.name })}
              >
                Rename folder
              </button>
            )}

            <div className="menu-divider" />

            {/* Its servers are not deleted with it; they go back to the top level of the rail. */}
            <button
              type="button"
              className="menu-item"
              onClick={() => {
                setMenu(null);
                setRenaming(null);
                onDeleteFolder(heldFolder.id);
              }}
            >
              Delete folder
            </button>
          </div>
        </>
      ) : null}
    </nav>
  );
};
