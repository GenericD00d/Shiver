/**
 * Shiver's server rail, drawn inside a server's page (Android has one webview) in a closed shadow
 * root, so Sharkord's Tailwind preflight and Shiver's styles cannot reach each other. It slides over
 * Sharkord's drawer and moves nothing else. The rail cannot call Shiver: picking a server or a menu
 * action navigates to Shiver's own page with a fragment, and reorders are polled by the core.
 */

import { defineHook } from '../../shared/web/bridge/dom';
import { markAllChannelsRead, SERVER_VIEW, SIDEBAR } from '../../shared/web/bridge/sharkord';
import type { RailCreate, RailEntry, RailFolder, RailMove, ShiverConfig } from './types';

export type Rail = {
  open: () => void;
  close: () => void;
  isOpen: () => boolean;
  /** the back button: closes a menu or toggles the rail; true when it used the press */
  back: () => boolean;
};

const RAIL_WIDTH = 68;
/** Sharkord's own swipe threshold */
const SWIPE_THRESHOLD = 50;
/** Tailwind's `md`, where Sharkord stops hiding its sidebar */
const WIDE_LAYOUT = 768;
/** a press held this long is a hold (menu or drag) */
export const HOLD_MS = 500;
/** how far a finger may drift and still be holding still */
export const HOLD_SLOP = 10;
/** how long after a hold its own click may still arrive and must be ignored */
export const CLICK_GRACE_MS = 700;

/** Moves and folders made in the rail since the core last looked (survive rail redraws). */
const pendingMoves: RailMove[] = [];
const pendingCreates: RailCreate[] = [];

let home = '';

/** Leaves for Shiver's own page, carrying at most an entry id in `hash`. */
function goHome(hash: string) {
  if (home) window.location.assign(home.replace(/#.*$/, '') + hash);
}

export function mountRail(shiver: ShiverConfig): Rail {
  home = shiver.home;
  document.getElementById('shiver-root')?.remove();

  const host = document.createElement('div');

  host.id = 'shiver-root';
  document.body.appendChild(host);

  const root = host.attachShadow({ mode: 'closed' });
  const scrim = el('div', 'scrim');
  const nav = el('nav', 'rail');
  const badges = new Map<string, HTMLElement>();
  const openFolders = new Set(shiver.folders.filter((folder) => folder.expanded).map((folder) => folder.id));
  let open = false;
  let counts: Record<string, number> = Object.fromEntries(shiver.rail.map((entry) => [entry.id, entry.unread]));

  root.appendChild(railStyle());
  nav.setAttribute('aria-label', 'Servers');

  const closeMenu = () => {
    const menu = root.querySelector('.menu');

    menu?.remove();

    return !!menu;
  };

  const setOpen = (next: boolean) => {
    open = next;

    if (!next) closeMenu();

    host.classList.toggle('open', next);
  };

  scrim.addEventListener('click', () => setOpen(false));

  const membersOf = (folderId: string) =>
    shiver.rail.filter((entry) => entry.folderId === folderId).sort((a, b) => a.position - b.position);

  /** Drops folders holding fewer than two servers, as the core does, so the rail agrees at once. */
  const dissolveThinFolders = () => {
    const doomed = new Set(shiver.folders.filter((folder) => membersOf(folder.id).length < 2).map((folder) => folder.id));

    if (!doomed.size) return;

    for (const entry of shiver.rail) {
      if (entry.folderId && doomed.has(entry.folderId)) entry.folderId = null;
    }

    for (const id of doomed) openFolders.delete(id);

    shiver.folders = shiver.folders.filter((folder) => !doomed.has(folder.id));
  };

  /** Folders and loose servers, in rail order (computed on every draw, since moves change it). */
  const topItems = () =>
    [
      ...shiver.folders.map((folder) => ({ kind: 'folder' as const, folder, position: folder.position })),
      ...shiver.rail.filter((entry) => !entry.folderId).map((entry) => ({ kind: 'server' as const, entry, position: entry.position }))
    ].sort((a, b) => a.position - b.position);

  const paintBadges = (unread: Record<string, number>) => {
    counts = unread;

    for (const [id, badge] of badges) {
      const count = unread[id] ?? 0;

      badge.textContent = count > 99 ? '99+' : String(count);
      badge.hidden = count === 0;
    }
  };

  const serverTile = (entry: RailEntry, inFolder: boolean) => {
    const active = entry.id === shiver.entryId;
    const badge = el('span', 'badge');

    badge.hidden = true;
    badges.set(entry.id, badge);

    const button = tile({
      label: entry.name,
      active,
      badge,
      reorderable: true,
      className: ['tile-server', inFolder ? 'in-folder' : '', entry.signedOut ? 'signed-out' : ''].filter(Boolean).join(' '),
      content: entry.icon ? image(entry.icon) : document.createTextNode(initials(entry.name)),
      onHold: (at) =>
        openServerMenu(root, entry, active, at, () => setOpen(false), shiver.folders, (folderId) => {
          // recorded for the core, and applied now so the rail moves at once
          pendingMoves.push({ serverId: entry.id, folderId });
          entry.folderId = folderId;

          if (folderId) openFolders.add(folderId);

          rebuild();
        }),
      onPick: () => (active ? setOpen(false) : goHome(`#open=${encodeURIComponent(entry.id)}`))
    });

    button.dataset.entryId = entry.id;

    return button;
  };

  const addTile = tile({ label: 'Add a server', className: 'add', content: plusIcon(), onPick: () => goHome('#add') });

  /** Redraws the servers and folders between the fixed tiles. */
  const rebuild = () => {
    dissolveThinFolders();

    for (const stale of nav.querySelectorAll('.tile-server, .tile-folder, .folder-contents')) stale.remove();

    badges.clear();

    for (const item of topItems()) {
      if (item.kind === 'server') {
        nav.insertBefore(serverTile(item.entry, false), addTile);

        continue;
      }

      const { folder } = item;
      const members = membersOf(folder.id);
      const isOpen = openFolders.has(folder.id);
      const button = tile({
        label: folder.name,
        className: isOpen ? 'tile-folder open' : 'tile-folder',
        content: folderIcon(members, isOpen),
        reorderable: true,
        onHold: (at) => openFolderMenu(root, folder, at),
        // open or shut for this page only; the stored state is set on Shiver's own screen
        onPick: () => {
          if (!openFolders.delete(folder.id)) openFolders.add(folder.id);

          rebuild();
        }
      });

      button.dataset.folderId = folder.id;
      nav.insertBefore(button, addTile);

      if (!isOpen) continue;

      const contents = el('div', 'folder-contents');

      contents.dataset.folderId = folder.id;

      for (const entry of members) contents.append(serverTile(entry, true));

      nav.insertBefore(contents, addTile);
    }

    paintBadges(counts);
  };

  nav.append(
    tile({
      label: 'Direct messages',
      content: messagesIcon(),
      // Shiver's own screen lists every server's DMs; drawing that list here would tell this server
      // who the user talks to elsewhere
      onPick: () => {
        setOpen(false);
        goHome('#dms');
      }
    }),
    el('div', 'divider'),
    addTile,
    el('div', 'spacer'),
    el('div', 'divider'),
    tile({ label: 'Shiver settings', content: settingsIcon(), onPick: () => goHome('#settings') })
  );

  installRailDrag(nav, shiver, openFolders, rebuild);
  root.append(scrim, nav);
  rebuild();
  defineHook('__SHIVER_UNREAD__', paintBadges);

  return {
    open: () => setOpen(true),
    close: () => setOpen(false),
    isOpen: () => open,
    back: () => {
      if (!closeMenu()) setOpen(!open);

      return true;
    }
  };
}

type TileOptions = {
  label: string;
  content: Node;
  active?: boolean;
  className?: string;
  /** overhangs the tile */
  badge?: HTMLElement;
  onPick: () => void;
  /** a long press (the rail's right-click) */
  onHold?: (at: number) => void;
  /** held and moved, it is dragged by `installRailDrag`, which dispatches `shiver-hold` when held still */
  reorderable?: boolean;
};

function tile({ label, content, active, className, badge, onPick, onHold, reorderable }: TileOptions) {
  const button = el('button', ['tile', active ? 'active' : '', className ?? ''].filter(Boolean).join(' '));

  button.setAttribute('type', 'button');
  button.setAttribute('aria-label', label);
  button.title = label;
  button.append(content);

  if (badge) button.append(badge);

  button.addEventListener('click', (event) => {
    // a press used for a menu or a drag is not also a tap (a time, not a flag, so it cannot go stale)
    const heldAt = Number(button.dataset.heldAt ?? 0);

    delete button.dataset.heldAt;

    if (heldAt && Date.now() - heldAt < CLICK_GRACE_MS) {
      event.preventDefault();
      event.stopPropagation();

      return;
    }

    onPick();
  });

  if (onHold && reorderable) {
    button.addEventListener('contextmenu', (event) => event.preventDefault());
    button.addEventListener('shiver-hold', (event) => onHold((event as CustomEvent<number>).detail));
  } else if (onHold) {
    installLongPress(button, onHold);
  }

  return button;
}

/** Calls `onHold(y)` when `target` is held still for `HOLD_MS` (its own timer: `contextmenu` is unreliable on touch). */
function installLongPress(target: HTMLElement, onHold: (at: number) => void) {
  let timer = 0;
  let startX = 0;
  let startY = 0;

  const cancel = () => {
    window.clearTimeout(timer);
    timer = 0;
  };

  target.addEventListener('contextmenu', (event) => event.preventDefault());
  target.addEventListener(
    'touchstart',
    (event) => {
      const touch = event.touches[0];

      if (!touch) return;

      startX = touch.clientX;
      startY = touch.clientY;
      timer = window.setTimeout(() => {
        target.dataset.heldAt = String(Date.now());
        onHold(startY);
      }, HOLD_MS);
    },
    { passive: true }
  );
  target.addEventListener(
    'touchmove',
    (event) => {
      const touch = event.touches[0];

      if (touch && (Math.abs(touch.clientX - startX) > HOLD_SLOP || Math.abs(touch.clientY - startY) > HOLD_SLOP)) cancel();
    },
    { passive: true }
  );
  target.addEventListener('touchend', cancel, { passive: true });
  target.addEventListener('touchcancel', cancel, { passive: true });
}

/**
 * Drag to reorder: hold a tile until it lifts, then move it. Tiles move in the DOM as the finger
 * passes them; resting on the middle of another tile groups the two (a new folder, or into that
 * folder), leaving an open folder's box takes a server out. Held still and released, the hold is
 * the tile's menu. The core reads the result through `__SHIVER_RAIL_STATE__`.
 */
function installRailDrag(nav: HTMLElement, shiver: ShiverConfig, openFolders: Set<string>, rebuild: () => void) {
  let timer = 0;
  let held: HTMLElement | null = null;
  let startY = 0;
  let holdAt = 0;
  let moved = false;
  /** the folder the held tile started in */
  let startFolder: string | null = null;
  /** the tile the held one would be grouped with */
  let onto: HTMLElement | null = null;
  /** how far the held tile is translated from its layout position */
  let carriedBy = 0;

  const clearOnto = () => {
    onto?.classList.remove('drop-onto');
    onto = null;
  };

  /** Keeps the held tile under the finger, measured from its layout position so reorders do not drift. */
  const carry = (y: number) => {
    if (!held) return;

    const box = held.getBoundingClientRect();

    carriedBy = y - (box.top + box.height / 2 - carriedBy);
    held.style.transform = `translateY(${carriedBy}px) scale(1.12)`;
  };

  const isTile = (node: Element): node is HTMLElement =>
    node instanceof HTMLElement && (node.classList.contains('tile-folder') || node.classList.contains('tile-server'));
  const tiles = () => [...nav.querySelectorAll<HTMLElement>('.tile-folder, .tile-server')];
  const topLevel = () => [...nav.children].filter(isTile);
  const folderOf = (tile: HTMLElement) =>
    tile.parentElement?.classList.contains('folder-contents') ? (tile.parentElement.dataset.folderId ?? null) : null;
  const entryOf = (id: string | undefined) => shiver.rail.find((entry) => entry.id === id);

  const setFolder = (serverId: string, folderId: string | null) => {
    const entry = entryOf(serverId);

    if (entry) entry.folderId = folderId;
  };

  /** Writes the on-screen order back into the rail's data before a redraw. */
  const syncPositions = () => {
    topLevel().forEach((tile, index) => {
      const folder = tile.dataset.folderId ? shiver.folders.find((candidate) => candidate.id === tile.dataset.folderId) : null;
      const entry = folder ? null : entryOf(tile.dataset.entryId);

      if (folder) folder.position = index;
      if (entry) entry.position = index;
    });

    for (const box of nav.querySelectorAll<HTMLElement>('.folder-contents')) {
      [...box.children].forEach((child, index) => {
        const entry = child instanceof HTMLElement ? entryOf(child.dataset.entryId) : undefined;

        if (entry) entry.position = index;
      });
    }
  };

  /** Keeps each open folder's box directly under its tile when tiles move. */
  const keepContentsWithFolders = () => {
    for (const tile of nav.querySelectorAll<HTMLElement>('.tile-folder')) {
      const id = tile.dataset.folderId;
      const contents = id ? nav.querySelector<HTMLElement>(`.folder-contents[data-folder-id="${CSS.escape(id)}"]`) : null;

      if (contents && tile.nextSibling !== contents) nav.insertBefore(contents, tile.nextSibling);
    }
  };

  defineHook('__SHIVER_RAIL_STATE__', () => ({
    order: topLevel()
      .map((tile) =>
        tile.dataset.folderId
          ? { kind: 'folder' as const, id: tile.dataset.folderId }
          : { kind: 'server' as const, id: tile.dataset.entryId ?? '' }
      )
      .filter((item) => !!item.id),
    moves: pendingMoves.splice(0),
    creates: pendingCreates.splice(0)
  }));

  const release = () => {
    if (held) {
      held.classList.remove('lifted');
      held.style.transform = '';
      held.style.transition = '';
    }

    window.clearTimeout(timer);
    timer = 0;
    carriedBy = 0;
    held = null;
    moved = false;
    startFolder = null;
    clearOnto();
  };

  nav.addEventListener(
    'touchstart',
    (event) => {
      const touch = event.touches[0];
      const tile = (event.target as Element | null)?.closest?.('.tile-server');

      release();

      if (!touch || !(tile instanceof HTMLElement)) return;

      startY = holdAt = touch.clientY;
      timer = window.setTimeout(() => {
        held = tile;
        startFolder = folderOf(tile);
        tile.classList.add('lifted');
        carry(startY);
      }, HOLD_MS);
    },
    { passive: true }
  );

  nav.addEventListener(
    'touchmove',
    (event) => {
      const touch = event.touches[0];

      if (!touch) return;

      // not lifted yet: a moving finger is scrolling the rail
      if (!held) {
        if (Math.abs(touch.clientY - startY) > HOLD_SLOP) release();

        return;
      }

      if (Math.abs(touch.clientY - startY) > HOLD_SLOP) moved = true;

      held.style.transition = 'none';
      carry(touch.clientY);

      // dragged out of the open folder's box: out of the folder, above or below it
      const group = held.parentElement?.classList.contains('folder-contents') ? held.parentElement : null;

      if (group) {
        const inside = group.getBoundingClientRect();

        if (touch.clientY < inside.top || touch.clientY > inside.bottom) {
          const folderTile = topLevel().find((node) => node.dataset.folderId === group.dataset.folderId);

          clearOnto();
          nav.insertBefore(held, touch.clientY < inside.top ? (folderTile ?? group) : group.nextSibling);
          held.classList.remove('in-folder');

          return;
        }
      }

      // folders only move among the top level
      const holdingFolder = held.classList.contains('tile-folder');
      const over = tiles().find((tile) => {
        if (tile === held || (holdingFolder && folderOf(tile))) return false;

        const box = tile.getBoundingClientRect();

        return touch.clientY >= box.top && touch.clientY <= box.bottom;
      });

      if (!over) return;

      const box = over.getBoundingClientRect();
      const offset = touch.clientY - box.top;
      // the middle of a tile means "group with it"; wider to leave than to enter, so it does not flicker
      const grouping = onto === over ? offset > box.height * 0.1 && offset < box.height * 0.9 : offset > box.height * 0.25 && offset < box.height * 0.75;

      if (!holdingFolder && grouping) {
        if (onto !== over) {
          clearOnto();
          onto = over;
          over.classList.add('drop-onto');
        }

        return;
      }

      clearOnto();

      // into the list of the tile passed, before or after it by its midpoint
      (over.parentElement ?? nav).insertBefore(held, touch.clientY < box.top + box.height / 2 ? over : over.nextSibling);
      held.classList.toggle('in-folder', !!folderOf(held));
      keepContentsWithFolders();
    },
    { passive: true }
  );

  nav.addEventListener(
    'touchend',
    () => {
      const tile = held;
      const wasDrag = moved;
      const was = startFolder;
      const target = onto;

      release();

      if (!tile) return;

      // the press is used either way, so its click must not open the server
      tile.dataset.heldAt = String(Date.now());

      const serverId = tile.dataset.entryId;

      if (wasDrag && serverId && target && target !== tile) {
        const intoFolder = target.dataset.folderId;
        const partner = target.dataset.entryId;

        if (intoFolder) {
          pendingMoves.push({ serverId, folderId: intoFolder });
          setFolder(serverId, intoFolder);
          openFolders.add(intoFolder);
        } else if (partner) {
          // the rail names the folder so it can draw it at once; the core keeps the id
          const id = crypto.randomUUID ? crypto.randomUUID() : `rail-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;

          pendingCreates.push({ id, name: 'New folder', memberIds: [partner, serverId] });
          syncPositions();
          shiver.folders.push({ id, name: 'New folder', position: topLevel().indexOf(target), expanded: true });
          setFolder(partner, id);
          setFolder(serverId, id);
          openFolders.add(id);
        }

        syncPositions();
        rebuild();

        return;
      }

      // moved into or out of a folder: the order alone cannot say so
      const landedIn = folderOf(tile);

      if (wasDrag && serverId && landedIn !== was) {
        pendingMoves.push({ serverId, folderId: landedIn });
        setFolder(serverId, landedIn);
        syncPositions();
        rebuild();
      }

      if (!wasDrag) tile.dispatchEvent(new CustomEvent('shiver-hold', { detail: holdAt }));
    },
    { passive: true }
  );

  nav.addEventListener('touchcancel', release, { passive: true });
}

/** A shut folder shows up to four member icons in a 2x2 grid; an open one is a slim bar. */
function folderIcon(members: RailEntry[], expanded: boolean) {
  if (expanded) return el('span', 'folder-open');

  const grid = el('span', 'folder-grid');

  for (const member of members.slice(0, 4)) {
    const cell = el('span', 'folder-cell');

    if (member.icon) cell.append(image(member.icon));
    else cell.textContent = initials(member.name).slice(0, 1);

    grid.append(cell);
  }

  return grid;
}

/** A menu beside the rail, closed by any tap outside it. */
function showMenu(root: ShadowRoot, at: number, items: (readonly [string, () => void] | 'divider')[]) {
  root.querySelector('.menu')?.remove();

  const menu = el('div', 'menu');

  menu.style.top = `${Math.max(8, at - 24)}px`;

  for (const item of items) {
    if (item === 'divider') {
      menu.append(el('div', 'menu-divider'));

      continue;
    }

    const [label, run] = item;
    const button = el('button', 'menu-item');

    button.setAttribute('type', 'button');
    button.textContent = label;
    button.addEventListener('click', () => {
      menu.remove();
      run();
    });
    menu.append(button);
  }

  root.append(menu);

  const dismiss = (event: Event) => {
    if (event.target instanceof Node && menu.contains(event.target)) return;

    menu.remove();
    root.removeEventListener('click', dismiss, true);
  };

  root.addEventListener('click', dismiss, true);
}

/** Folders are renamed and deleted on Shiver's own screen, which a server's page cannot reach. */
function openFolderMenu(root: ShadowRoot, folder: RailFolder, at: number) {
  showMenu(root, at, [[`Manage "${folder.name}"`, () => goHome('#settings')]]);
}

/**
 * A server tile's menu. Marking all read needs that server's own client, so it is offered only for
 * the server on screen; everything that changes Shiver's records goes through Shiver's page, where
 * destructive actions are confirmed.
 */
function openServerMenu(
  root: ShadowRoot,
  entry: RailEntry,
  active: boolean,
  at: number,
  closeRail: () => void,
  folders: RailFolder[],
  moved: (folderId: string | null) => void
) {
  const id = encodeURIComponent(entry.id);
  const moves = [
    ...folders.filter((folder) => folder.id !== entry.folderId).map((folder) => [`Move to "${folder.name}"`, () => moved(folder.id)] as const),
    ...(entry.folderId ? [['Take out of folder', () => moved(null)] as const] : [])
  ];

  showMenu(root, at, [
    active
      ? ([
          'Mark all as read',
          () => {
            markAllChannelsRead();
            closeRail();
          }
        ] as const)
      : (['Open', () => goHome(`#open=${id}`)] as const),
    ['Refresh name and icon', () => goHome(`#do=refresh:${id}`)],
    'divider',
    ...moves,
    ...(moves.length ? ['divider' as const] : []),
    ['Log out', () => goHome(`#do=logout:${id}`)],
    ['Forget my password', () => goHome(`#do=forgetpw:${id}`)],
    ['Remove from Shiver', () => goHome(`#do=remove:${id}`)]
  ]);
}

/** Whether Sharkord's drawer is on screen (always, above `md`). */
function drawerIsOpen() {
  const sidebar = document.querySelector(SIDEBAR);

  return sidebar instanceof HTMLElement ? sidebar.getBoundingClientRect().right > 1 : window.innerWidth >= WIDE_LAYOUT;
}

/** Opens Sharkord's drawer by performing the swipe it listens for (it has no API). */
export function openDrawer() {
  if (drawerIsOpen()) return;

  const view = document.querySelector(SERVER_VIEW) ?? document.body;

  try {
    const at = (clientX: number): TouchEventInit => {
      const touch = new Touch({ identifier: 1, target: view, clientX, clientY: 300 });

      return { touches: [touch], changedTouches: [touch], bubbles: true };
    };

    view.dispatchEvent(new TouchEvent('touchstart', at(8)));
    view.dispatchEvent(new TouchEvent('touchmove', at(200)));
    view.dispatchEvent(new TouchEvent('touchend', { ...at(200), touches: [] }));
  } catch {
    // no Touch constructor
  }
}

/**
 * Adds the level above Sharkord's drawer, deciding in the capture phase before Sharkord sees the
 * gesture: drawer closed, a right swipe is Sharkord's; drawer open (swipe starting over it), it
 * opens the rail; rail open, a left swipe closes it. Only `touchend` is ever swallowed. On a page
 * without a sidebar (sign-in, errors) any right swipe opens the rail.
 */
export function installGestures(rail: Rail) {
  let startX = 0;
  let startY = 0;
  let lastX = 0;
  let lastY = 0;
  const passive = { capture: true, passive: true } as const;

  const track = (event: TouchEvent, start: boolean) => {
    const touch = event.touches[0];

    // synthesised touches (`openDrawer`) are not the user's
    if (!event.isTrusted || !touch) return;

    lastX = touch.clientX;
    lastY = touch.clientY;

    if (start) {
      startX = lastX;
      startY = lastY;
    }
  };

  window.addEventListener('touchstart', (event) => track(event, true), passive);
  window.addEventListener('touchmove', (event) => track(event, false), passive);
  window.addEventListener(
    'touchend',
    (event) => {
      if (!event.isTrusted) return;

      const dx = lastX - startX;

      if (Math.abs(dx) < SWIPE_THRESHOLD || Math.abs(dx) <= Math.abs(lastY - startY)) return;

      if (rail.isOpen()) {
        if (dx < 0) rail.close();

        event.stopPropagation();

        return;
      }

      if (dx < 0) return;

      const sidebar = document.querySelector(SIDEBAR);

      if (!(sidebar instanceof HTMLElement)) {
        rail.open();

        return;
      }

      if (drawerIsOpen() && startX <= Math.max(sidebar.getBoundingClientRect().right, 0) + 40) {
        rail.open();
        event.stopPropagation();
      }
    },
    { capture: true }
  );
}

/* ── small parts ── */

function el(tag: string, className: string) {
  const node = document.createElement(tag);

  node.className = className;

  return node;
}

/** Not draggable: the platform's image drag would cancel Shiver's. */
function image(src: string) {
  const img = document.createElement('img');

  img.src = src;
  img.alt = '';
  img.draggable = false;

  return img;
}

/** Matches the React rail's fallback. */
function initials(name: string) {
  return (
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word[0]?.toUpperCase() ?? '')
      .join('') || '?'
  );
}

function svg(paths: string) {
  const node = document.createElementNS('http://www.w3.org/2000/svg', 'svg');

  for (const [name, value] of Object.entries({
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    'stroke-width': '2',
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round'
  })) {
    node.setAttribute(name, value);
  }

  node.innerHTML = paths;

  return node;
}

const messagesIcon = () => svg('<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />');
const plusIcon = () => svg('<path d="M12 5v14M5 12h14" />');
const settingsIcon = () =>
  svg(
    '<circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1" />'
  );

/**
 * The rail's styles: Sharkord's dark palette as fallbacks for Shiver's `--shiver-*` properties,
 * which `applyTheme` sets on Shiver's page and which inherit into the shadow root. The `.badge` rule
 * must match `.rail-badge` in `mobile/src/styles.css` (`scripts/check-rails.py`). No backticks in
 * comments here: this is a template literal.
 */
function railStyle() {
  const style = document.createElement('style');

  style.textContent = `
:host { position: fixed; inset: 0; z-index: 2147483647; pointer-events: none;
  --shiver-slide: 300ms; --shiver-ease: cubic-bezier(0.4, 0, 0.2, 1); --shiver-rail-width: ${RAIL_WIDTH}px;
  font: 600 14px/1.2 system-ui, -apple-system, "Segoe UI", sans-serif; }
:host(.open) .scrim { opacity: 1; pointer-events: auto; }
:host(.open) .rail { transform: translateX(0); }

.scrim { position: absolute; inset: 0; background: rgb(0 0 0 / 45%); opacity: 0;
  transition: opacity var(--shiver-slide) var(--shiver-ease); }

.rail { position: absolute; top: 0; bottom: 0; left: 0; width: var(--shiver-rail-width); box-sizing: border-box;
  padding: calc(10px + env(safe-area-inset-top, 0px)) 0 calc(10px + env(safe-area-inset-bottom, 0px));
  display: flex; flex-direction: column; align-items: center; gap: 8px;
  background: var(--shiver-rail, #171717); border-right: 1px solid var(--shiver-border, rgb(255 255 255 / 10%));
  overflow-y: auto; overflow-x: hidden; scrollbar-width: none;
  transform: translateX(-100%); pointer-events: auto; transition: transform var(--shiver-slide) var(--shiver-ease); }
.rail::-webkit-scrollbar { display: none; }

/* unselectable: a selection starting on a hold would cancel the drag */
.tile { position: relative; width: 48px; height: 48px; flex: 0 0 48px; border: none; padding: 0; border-radius: 16px;
  background: var(--shiver-surface, #262626); color: var(--shiver-text, #fafafa);
  display: grid; place-items: center; font: inherit; font-size: 16px;
  user-select: none; -webkit-user-select: none; -webkit-touch-callout: none;
  transition: border-radius 120ms ease, background 120ms ease; }
.tile:active { background: var(--shiver-surface-hover, #333333); border-radius: 14px; }
.tile.active { border-radius: 14px; box-shadow: inset 0 0 0 2px var(--shiver-accent, #e5e5e5); }
.tile.add { color: var(--shiver-text, #e5e5e5); font-size: 24px; }
.tile img { width: 100%; height: 100%; object-fit: cover; border-radius: inherit; pointer-events: none; -webkit-user-drag: none; }
.tile svg { width: 16px; height: 16px; display: block; }

.tile.tile-folder { background: var(--shiver-surface-dim, #1f1f1f); padding: 4px; }
.folder-grid { display: grid; grid-template-columns: 1fr 1fr; grid-template-rows: 1fr 1fr; gap: 2px; width: 100%; height: 100%; }
.folder-cell { border-radius: 5px; background: var(--shiver-surface-hover, #333333); color: var(--shiver-text, #e5e5e5);
  display: grid; place-items: center; font-size: 9px; font-weight: 700; overflow: hidden; }
.folder-cell img { width: 100%; height: 100%; object-fit: cover; pointer-events: none; -webkit-user-drag: none; }
.tile.tile-folder.open { height: 20px; flex: 0 0 20px; border-radius: 10px; padding: 0; }
.folder-open { width: 16px; height: 3px; border-radius: 2px; background: var(--shiver-text-dim, #8a8a8a); }
.folder-contents { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 4px 0; }
.tile.in-folder { width: 40px; height: 40px; flex: 0 0 40px; border-radius: 13px; font-size: 14px; }

.badge { position: absolute; top: -2px; right: -2px; min-width: 18px; width: 18px; height: 18px;
  padding: 0; border-radius: 50%; text-align: center;
  background: var(--shiver-text, #fafafa);
  color: var(--shiver-rail, #171717);
  font-size: 9px; line-height: 18px; font-weight: 700; pointer-events: none;
  box-shadow: 0 0 0 1px var(--shiver-rail, #171717); }

.menu { position: absolute; left: calc(var(--shiver-rail-width) + 8px); min-width: 200px; padding: 6px; border-radius: 12px;
  border: 1px solid var(--shiver-border, rgb(255 255 255 / 12%));
  background: var(--shiver-surface-dim, #1f1f1f); color: var(--shiver-text, #fafafa);
  box-shadow: 0 12px 32px rgb(0 0 0 / 55%); pointer-events: auto; z-index: 1; }
.menu-item { display: block; width: 100%; padding: 11px 12px; border: none; border-radius: 8px;
  background: none; color: inherit; font: inherit; text-align: left; }
.menu-item:active { background: var(--shiver-surface-hover, #333333); }
.menu-divider { height: 1px; margin: 6px 4px; background: var(--shiver-border, rgb(255 255 255 / 12%)); }

.tile.lifted { transform: scale(1.12); box-shadow: 0 8px 20px rgb(0 0 0 / 55%); opacity: 0.92; transition: transform 120ms ease; z-index: 2; }
.tile.signed-out { opacity: 0.55; box-shadow: 0 0 0 2px var(--shiver-danger, #ff6467); }
.tile.drop-onto { box-shadow: 0 0 0 2px var(--shiver-accent, #e5e5e5); }

.spacer { flex: 1 1 auto; min-height: 8px; }
.divider { width: 32px; height: 1px; flex: 0 0 1px; background: var(--shiver-border, rgb(255 255 255 / 10%)); }
`;

  return style;
}
