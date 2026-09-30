import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { MessagesIcon, PlusIcon, SettingsIcon } from './icons';
import { byPosition, type DropZone, initials, membersOf, railDrop, type RailRef, type RailStep } from '../rail';
import type { Folder } from '../types';
import './rail.css';

/** A server as the rail draws it. */
export type RailServer = {
  id: string;
  name: string;
  /** its logo, as a url or `data:` uri; the initials without one */
  icon: string | null;
  folderId: string | null;
  position: number;
  accountLabel?: string | null;
};

/** One of Shiver's own screens, whose tile is ringed while it is up. */
export type RailScreen = 'dms' | 'add' | 'settings';

type Props = {
  servers: readonly RailServer[];
  folders: readonly Folder[];
  /** the server on screen, ringed unless one of Shiver's screens is up */
  activeId: string | null;
  screen: RailScreen | null;
  /** unread per server id; a server missing from it has none */
  unread: Record<string, number>;
  /** servers not answering, marked red */
  offline?: readonly string[];
  onOpen: (id: string) => void;
  onOpenDms: () => void;
  onAdd: () => void;
  onSettings: () => void;
  onToggleFolder: (id: string, expanded: boolean) => void;
  /** a drag ended: the calls to make, in order (`railDrop`) */
  onDrop: (steps: RailStep[]) => void;
  /** a right click, or a touch held still: the menu for this tile, beside it */
  onMenu: (target: RailRef, at: { x: number; y: number }) => void;
  /** drawn above the settings tile (the desktop client's call controls) */
  children?: ReactNode;
};

/** Matches the bell, the other place Shiver counts unread. */
const badgeLabel = (count: number) => (count > 99 ? '99+' : String(count));

/** The four shown in a collapsed folder's tile. */
const FOLDER_PREVIEW_COUNT = 4;
/** How long a finger has to rest on a tile to pick it up (and, released still, to mean "menu"). */
const HOLD_MS = 500;
/** How far a finger may drift during that hold and still be a press rather than a scroll. */
const HOLD_SLOP = 10;
/** How far a mouse has to move with the button down to pick a tile up rather than click it. */
const MOUSE_SLOP = 4;
/** The band at a tile's centre that merges rather than reorders: fixed for a mouse, half a tile for a finger. */
const MERGE_BAND_PX = 14;
/** Within this of the rail's ends, a carried tile scrolls it. */
const EDGE_PX = 32;

type Hover = { key: string; zone: DropZone } | null;

const refOf = (key: string): RailRef => {
  const split = key.indexOf(':');

  return { kind: key.slice(0, split) === 'folder' ? 'folder' : 'server', id: key.slice(split + 1) };
};

/** A red mark over a server that is not answering (cut out, so its icon shows through; per-server mask id). */
const OfflineOverlay = ({ id }: { id: string }) => {
  const maskId = `shiver-offline-${id}`;

  return (
    <span className="rail-offline" aria-hidden="true">
      <svg viewBox="0 0 48 48" width="48" height="48">
        <mask id={maskId}>
          <rect width="48" height="48" fill="#fff" />
          <rect x="21.5" y="11" width="5" height="17" rx="2.5" fill="#000" />
          <circle cx="24" cy="34.5" r="2.75" fill="#000" />
        </mask>

        {/* the icon is dimmed first, and only then reddened. without it the mark is cut out onto
            whatever the icon happens to be — and on a light or warm icon a red-on-orange
            exclamation is close to invisible, which is the one thing this must not be. */}
        <rect width="48" height="48" fill="#000" opacity="0.5" />
        <rect width="48" height="48" fill="#ef4444" opacity="0.7" mask={`url(#${maskId})`} />
      </svg>
    </span>
  );
};

/**
 * Shiver's rail, as both clients draw it: direct messages, the servers and folders, add, settings.
 * Tiles are picked up with Pointer Events, so one drag serves a mouse (it lifts after a small move)
 * and a finger (after a hold, which released still opens the menu instead). A dropped tile goes
 * before or after the one under it, or into it: a folder, or a new one made of the pair.
 */
export const Rail = ({
  servers,
  folders,
  activeId,
  screen,
  unread,
  offline = [],
  onOpen,
  onOpenDms,
  onAdd,
  onSettings,
  onToggleFolder,
  onDrop,
  onMenu,
  children
}: Props) => {
  const rail = useRef<HTMLElement>(null);
  /** every tile on screen by `server:<id>` or `folder:<id>`, for hit-testing a drag */
  const tiles = useRef(new Map<string, HTMLButtonElement>());
  const [lifted, setLifted] = useState<string | null>(null);
  const [hover, setHoverState] = useState<Hover>(null);
  const hoverRef = useRef<Hover>(null);
  /** the press under way: where it started, and whether it has picked its tile up */
  const press = useRef<{
    key: string;
    pointerId: number;
    touch: boolean;
    x: number;
    y: number;
    lastY: number;
    scroll: number;
    lifted: boolean;
    moved: boolean;
    timer: number;
  } | null>(null);
  /** the click that ends a press which opened a menu or moved a tile is not also a tap */
  const swallowClick = useRef(0);
  const frame = useRef(0);

  const setHover = (next: Hover) => {
    if (next?.key === hoverRef.current?.key && next?.zone === hoverRef.current?.zone) return;

    hoverRef.current = next;
    setHoverState(next);
  };

  const items = useMemo(
    () =>
      byPosition([
        ...servers
          .filter((server) => !server.folderId)
          .map((server) => ({ kind: 'server' as const, position: server.position, server })),
        ...folders.map((folder) => ({
          kind: 'folder' as const,
          position: folder.position,
          folder,
          contents: membersOf(servers, folder.id)
        }))
      ]),
    [folders, servers]
  );

  // the latest props, for the window listeners a press installs once
  const latest = useRef({ servers, folders, onDrop, onMenu });

  latest.current = { servers, folders, onDrop, onMenu };

  const menuAt = useCallback((key: string) => {
    const box = tiles.current.get(key)?.getBoundingClientRect();

    return box ? { x: box.right + 8, y: box.top } : { x: 0, y: 0 };
  }, []);

  /** Where a drop would land for a pointer at `y`, if anywhere worth marking. */
  const aim = useCallback((source: string, y: number, touch: boolean): Hover => {
    const { servers, folders } = latest.current;
    let best: { key: string; box: DOMRect; distance: number } | null = null;

    for (const [key, node] of tiles.current) {
      if (key === source) continue;

      const box = node.getBoundingClientRect();
      // the gaps between tiles belong to the nearer one, so no pixel of the column is dead
      const distance = y < box.top ? box.top - y : y > box.bottom ? y - box.bottom : 0;

      if (distance <= 5 && (!best || distance < best.distance)) best = { key, box, distance };
    }

    if (!best) return null;

    const target = refOf(best.key);
    const middle = best.box.top + best.box.height / 2;
    const band = touch ? best.box.height / 2 : MERGE_BAND_PX;
    let zone: DropZone = y < middle ? 'before' : 'after';

    if (Math.abs(y - middle) <= band / 2) {
      const into = railDrop(servers, folders, refOf(source), target, 'into');

      if (into.length > 0) zone = 'into';
    }

    return railDrop(servers, folders, refOf(source), target, zone).length > 0 ? { key: best.key, zone } : null;
  }, []);

  /** Keeps the lifted tile under the pointer, allowing for the rail having scrolled under it. */
  const carry = useCallback(() => {
    const current = press.current;
    const node = current && tiles.current.get(current.key);

    if (!current || !node || !rail.current) return;

    const scrolled = rail.current.scrollTop - current.scroll;

    node.style.transform = `translateY(${current.lastY - current.y + scrolled}px) scale(1.12)`;
  }, []);

  const release = useCallback(() => {
    const current = press.current;

    press.current = null;
    window.clearTimeout(current?.timer);
    window.cancelAnimationFrame(frame.current);

    const node = current && tiles.current.get(current.key);

    if (node) node.style.transform = '';

    setLifted(null);
    setHover(null);
  }, []);

  // a carried tile near either end of the rail scrolls it, for as long as it is held there
  const scrollLoop = useCallback(() => {
    const current = press.current;
    const node = rail.current;

    if (!current?.lifted || !node) return;

    const box = node.getBoundingClientRect();
    const step = current.lastY < box.top + EDGE_PX ? -6 : current.lastY > box.bottom - EDGE_PX ? 6 : 0;

    if (step) {
      node.scrollTop += step;
      carry();
      setHover(aim(current.key, current.lastY, current.touch));
    }

    frame.current = window.requestAnimationFrame(scrollLoop);
  }, [aim, carry]);

  const lift = useCallback(() => {
    const current = press.current;

    if (!current) return;

    current.lifted = true;
    setLifted(current.key);
    carry();
    frame.current = window.requestAnimationFrame(scrollLoop);
  }, [carry, scrollLoop]);

  useEffect(() => {
    const onMove = (event: PointerEvent) => {
      const current = press.current;

      if (!current || event.pointerId !== current.pointerId) return;

      const drift = Math.max(Math.abs(event.clientX - current.x), Math.abs(event.clientY - current.y));

      current.lastY = event.clientY;

      if (!current.lifted) {
        if (current.touch) {
          // before the hold lands, a finger that wanders is a scroll rather than a press
          if (drift > HOLD_SLOP) release();
        } else if (drift > MOUSE_SLOP) {
          current.moved = true;
          lift();
        }

        return;
      }

      if (drift > HOLD_SLOP) current.moved = true;

      carry();
      setHover(aim(current.key, event.clientY, current.touch));
    };

    const onUp = (event: PointerEvent) => {
      const current = press.current;

      if (!current || event.pointerId !== current.pointerId) return;

      const target = hoverRef.current;

      if (current.lifted) {
        swallowClick.current = Date.now();

        // held still, a finger meant the menu; moved, the order is what matters
        if (current.touch && !current.moved) {
          latest.current.onMenu(refOf(current.key), menuAt(current.key));
        } else if (target) {
          const { servers, folders, onDrop } = latest.current;

          onDrop(railDrop(servers, folders, refOf(current.key), refOf(target.key), target.zone));
        }
      }

      release();
    };

    // the platform took the gesture (a scroll, a system gesture): nothing is dropped
    const onCancel = (event: PointerEvent) => {
      if (press.current?.pointerId === event.pointerId) release();
    };

    // A lifted tile follows the finger rather than the rail scrolling under it. This listener is
    // the rail's own and blocking from the start, since a touch decides at its start whether its
    // moves can be cancelled.
    const onTouchMove = (event: TouchEvent) => {
      if (press.current?.lifted) event.preventDefault();
    };
    const node = rail.current;

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onCancel);
    node?.addEventListener('touchmove', onTouchMove, { passive: false });

    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
      node?.removeEventListener('touchmove', onTouchMove);
      release();
    };
  }, [aim, carry, lift, menuAt, release]);

  /** What every draggable tile takes: the press, the menu, and the click that is not a press. */
  const handlers = (key: string, onTap: () => void) => ({
    ref: (node: HTMLButtonElement | null) => {
      if (node) tiles.current.set(key, node);
      else tiles.current.delete(key);
    },
    onPointerDown: (event: React.PointerEvent) => {
      // the right button is the menu's, which `contextmenu` brings
      if (event.button !== 0 || press.current) return;

      const touch = event.pointerType === 'touch';

      press.current = {
        key,
        pointerId: event.pointerId,
        touch,
        x: event.clientX,
        y: event.clientY,
        lastY: event.clientY,
        scroll: rail.current?.scrollTop ?? 0,
        lifted: false,
        moved: false,
        timer: touch ? window.setTimeout(lift, HOLD_MS) : 0
      };
    },
    onContextMenu: (event: React.MouseEvent) => {
      event.preventDefault();

      // a held finger brings this too, and its menu comes from the hold instead
      if (press.current?.touch) return;

      onMenu(refOf(key), menuAt(key));
    },
    onClick: () => {
      if (Date.now() - swallowClick.current < HOLD_MS) return;

      onTap();
    }
  });

  const zoneClass = (key: string, into: string) => {
    if (hover?.key !== key) return '';

    return hover.zone === 'into' ? ` ${into}` : ` insert-${hover.zone}`;
  };

  const renderServer = (server: RailServer, inFolder: boolean) => {
    const key = `server:${server.id}`;
    const count = unread[server.id] ?? 0;
    const down = offline.includes(server.id);
    const classes = [
      'rail-item',
      server.id === activeId && !screen ? 'active' : '',
      inFolder ? 'in-folder' : '',
      lifted === key ? 'lifted' : ''
    ]
      .filter(Boolean)
      .join(' ');

    return (
      // the slot reaches into the gaps between tiles, so the rail looks the same and a drop there lands
      <div className="rail-slot" key={server.id}>
        <button
          type="button"
          className={classes + zoneClass(key, 'merge-target')}
          title={
            down
              ? `${server.name} — not responding`
              : server.accountLabel
                ? `${server.name} (${server.accountLabel})`
                : server.name
          }
          {...handlers(key, () => onOpen(server.id))}
        >
          {server.icon ? <img src={server.icon} alt="" draggable={false} /> : initials(server.name)}

          {count > 0 ? <span className="rail-badge">{badgeLabel(count)}</span> : null}

          {down ? <OfflineOverlay id={server.id} /> : null}
        </button>
      </div>
    );
  };

  const renderFolder = (folder: Folder, contents: RailServer[]) => {
    const key = `folder:${folder.id}`;
    // only while collapsed: an expanded folder shows each member's own badge just below, and
    // showing both would count the same messages twice on screen
    const count = folder.expanded ? 0 : contents.reduce((total, server) => total + (unread[server.id] ?? 0), 0);
    const classes = ['rail-folder-tile', folder.expanded ? 'expanded' : '', lifted === key ? 'lifted' : '']
      .filter(Boolean)
      .join(' ');

    return (
      <div className="rail-folder" key={folder.id}>
        <div className="rail-slot">
          <button
            type="button"
            className={classes + zoneClass(key, 'drop-target')}
            title={folder.name}
            aria-expanded={folder.expanded}
            {...handlers(key, () => onToggleFolder(folder.id, !folder.expanded))}
          >
            {/* collapsed, the tile *is* the preview: up to four member icons in a 2x2 grid */}
            {folder.expanded ? (
              <span className="rail-folder-open" aria-hidden="true" />
            ) : (
              <span className="rail-folder-preview">
                {contents.slice(0, FOLDER_PREVIEW_COUNT).map((server) => (
                  <span className="rail-folder-mini" key={server.id}>
                    {server.icon ? <img src={server.icon} alt="" draggable={false} /> : initials(server.name).slice(0, 1)}
                  </span>
                ))}
              </span>
            )}

            {count > 0 ? <span className="rail-badge">{badgeLabel(count)}</span> : null}
          </button>
        </div>

        {folder.expanded ? (
          <div className="rail-folder-contents">{contents.map((server) => renderServer(server, true))}</div>
        ) : null}
      </div>
    );
  };

  return (
    <nav ref={rail} className={lifted ? 'rail dragging' : 'rail'} aria-label="Servers">
      <button
        type="button"
        className={screen === 'dms' ? 'rail-item active' : 'rail-item'}
        title="Direct messages"
        onClick={onOpenDms}
      >
        <MessagesIcon />
      </button>

      <div className="rail-divider" />

      {items.map((item) =>
        item.kind === 'folder' ? renderFolder(item.folder, item.contents) : renderServer(item.server, false)
      )}

      <button
        type="button"
        className={screen === 'add' ? 'rail-item rail-add active' : 'rail-item rail-add'}
        title="Add a server"
        onClick={onAdd}
      >
        <PlusIcon />
      </button>

      <div className="rail-spacer" />

      {/* above the settings tile, so the call, when there is one, is the last thing the eye reaches on
          the way down the rail */}
      {children}

      <div className="rail-divider" />

      <button
        type="button"
        className={screen === 'settings' ? 'rail-item active' : 'rail-item'}
        title="Shiver settings"
        onClick={onSettings}
      >
        <SettingsIcon />
      </button>
    </nav>
  );
};
