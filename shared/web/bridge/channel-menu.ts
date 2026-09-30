/**
 * Shiver's channel menu, for a right-click (desktop) or a long press (mobile) on a sidebar channel:
 * mute or unmute it, and mark every channel read. Sharkord has a channel menu of its own only for
 * channel managers and voice channels; when that opens, Shiver's items join it. Otherwise, after a
 * short grace, Shiver draws its own in a closed shadow root.
 *
 * The bridges own the gesture and report it: `channelPressed` when a press begins, and
 * `requestChannelMenu` when it asks for a menu.
 */

import { ensureStyle } from './dom';
import { channelOfRow, pressEscape, type SharkordChannel } from './sharkord';

export type ChannelMutes = { has: (channelId: number) => boolean; toggle: (channelId: number) => void };

export type ChannelMenuSetup = {
  mutes: ChannelMutes;
  markAllRead: () => void;
  /** finger-sized items, and a longer wait for Sharkord's menu (which a long press opens late) */
  touch: boolean;
  /** told when Sharkord's menu took Shiver's items, which may be before the gesture asked */
  onJoined?: () => void;
};

/** How long after a press a newly added menu is taken to belong to it. */
const PRESS_WINDOW_MS = 1500;
/** Shiver's items in Sharkord's menu */
const ITEM_CLASS = 'shiver-menu-item';
const HOST_ID = 'shiver-channel-menu';

let setup: ChannelMenuSetup | null = null;
/** the row the last press began on, and when */
let press: { row: Element; at: number } | null = null;
let waiting = 0;
let host: HTMLElement | null = null;
let release: (() => void) | null = null;

/** Sharkord's open (Radix) menu, if one is on screen; closed menus can linger in the document. */
function openMenuOnScreen(): HTMLElement | null {
  for (const menu of document.querySelectorAll<HTMLElement>('[role="menu"]')) {
    if (menu.dataset.state === 'closed') continue;

    const box = menu.getBoundingClientRect();

    if (box.width > 0 && box.height > 0) return menu;
  }

  return null;
}

/** The `[role="menu"]` a mutation added, if any. */
function addedMenu(records: MutationRecord[]): HTMLElement | null {
  for (const record of records) {
    for (const node of record.addedNodes) {
      if (!(node instanceof HTMLElement)) continue;

      const menu = node.matches('[role="menu"]') ? node : node.querySelector<HTMLElement>('[role="menu"]');

      if (menu) return menu;
    }
  }

  return null;
}

/** Watches for Sharkord's own menu opening after a press. Once per page. */
export function installChannelMenu(next: ChannelMenuSetup) {
  if (setup) return;

  setup = next;
  ensureStyle('shiver-channel-menu-style').textContent =
    `.${ITEM_CLASS}:hover, .${ITEM_CLASS}:focus { background-color: var(--accent); color: var(--accent-foreground); }`;

  new MutationObserver((records) => {
    if (!press || Date.now() - press.at > PRESS_WINDOW_MS) return;

    const menu = addedMenu(records);
    const channel = menu && channelOfRow(press.row);

    if (!menu || !channel) return;

    join(menu, channel);
    forgetPress();
    closeChannelMenu();
    setup?.onJoined?.();
  }).observe(document.body, { childList: true, subtree: true });
}

function forgetPress() {
  window.clearTimeout(waiting);
  waiting = 0;
  press = null;
}

/**
 * A press began on a sidebar channel `row`, so a Sharkord menu appearing soon after is for it; or
 * (null) somewhere else, which closes Shiver's menu and forgets the last press.
 */
export function channelPressed(row: Element | null) {
  forgetPress();
  closeChannelMenu();

  if (row) press = { row, at: Date.now() };
}

/**
 * The press on `row` asks for a menu at (x, y): Sharkord's gets Shiver's items if it is open by the
 * end of the grace, else Shiver draws its own. False when the row is no channel Shiver knows.
 */
export function requestChannelMenu(row: Element, x: number, y: number) {
  const channel = channelOfRow(row);

  if (!setup || !channel) return false;

  window.clearTimeout(waiting);
  waiting = window.setTimeout(
    () => {
      forgetPress();

      // Radix moves an already-open menu rather than adding one, so the observer never sees it
      const open = openMenuOnScreen();

      if (open) join(open, channel);
      else draw(channel, x, y);
    },
    // a right-click opens Sharkord's menu at once; a long press opens it after Shiver's hold
    setup.touch ? 450 : 250
  );

  return true;
}

/**
 * Adds Shiver's items to one of Sharkord's own menus, styled like its siblings. Replaces those left
 * from an earlier open, since Radix reuses its menu.
 */
function join(menu: HTMLElement, channel: SharkordChannel) {
  if (!setup) return;

  const { mutes, markAllRead } = setup;

  for (const stale of menu.querySelectorAll(`.${ITEM_CLASS}`)) stale.remove();

  addItem(menu, mutes.has(channel.id) ? 'Unmute in Shiver' : 'Mute in Shiver', () => mutes.toggle(channel.id));
  addItem(menu, 'Mark all as read', markAllRead);
}

function addItem(menu: HTMLElement, label: string, run: () => void) {
  const sibling = menu.querySelector<HTMLElement>(`[role="menuitem"]:not(.${ITEM_CLASS})`);
  const item = document.createElement('div');

  item.setAttribute('role', 'menuitem');
  item.tabIndex = -1;
  item.className = `${sibling?.className ?? ''} ${ITEM_CLASS}`.trim();
  item.textContent = label;

  item.addEventListener('mouseenter', () => item.focus());
  item.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    run();
    // lets Sharkord close its menu as it would for any item
    pressEscape();
  });

  menu.appendChild(item);
}

const menuStyles = (touch: boolean) => `
:host { position: fixed; inset: 0; z-index: 2147483646; }
.sheet { position: absolute; inset: 0; }
.menu { position: absolute; box-sizing: border-box; min-width: ${touch ? 200 : 180}px; max-width: min(320px, 80vw);
  padding: ${touch ? 6 : 4}px; border-radius: ${touch ? 12 : 8}px; border: 1px solid var(--border, rgb(255 255 255 / 12%));
  background: var(--popover, #1f1f1f); color: var(--popover-foreground, #fafafa); box-shadow: 0 12px 32px rgb(0 0 0 / 55%);
  font: 500 ${touch ? 14 : 13}px/1.2 system-ui, -apple-system, "Segoe UI", sans-serif; }
.item { display: block; width: 100%; padding: ${touch ? '11px 12px' : '8px 10px'}; border: none; border-radius: ${touch ? 8 : 6}px;
  background: none; color: inherit; font: inherit; text-align: left; cursor: default; outline: none;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.item:hover, .item:focus-visible, .item:active { background: var(--accent, #333333); color: var(--accent-foreground, inherit); }
`;

/**
 * Shiver's own menu, over a sheet that closes it. Only a press that began after it opened counts:
 * the right-click or finger that opened it can still be lifting over it.
 */
function draw(channel: SharkordChannel, x: number, y: number) {
  if (!setup) return;

  const { mutes, markAllRead, touch } = setup;

  closeChannelMenu();

  const element = document.createElement('div');
  const root = element.attachShadow({ mode: 'closed' });
  const sheet = document.createElement('div');
  const menu = document.createElement('div');
  let armed = false;

  /** a click from a press made inside, or from the keyboard (which has no press) */
  const deliberate = (event: MouseEvent) => armed || event.detail === 0;

  element.id = HOST_ID;
  root.innerHTML = `<style>${menuStyles(touch)}</style>`;
  sheet.className = 'sheet';
  menu.className = 'menu';
  menu.setAttribute('role', 'menu');

  const items: [string, () => void][] = [
    [`${mutes.has(channel.id) ? 'Unmute' : 'Mute'} #${channel.name}`, () => mutes.toggle(channel.id)],
    ['Mark all as read', markAllRead]
  ];

  for (const [label, run] of items) {
    const item = document.createElement('button');

    item.type = 'button';
    item.className = 'item';
    item.setAttribute('role', 'menuitem');
    item.textContent = label;
    item.addEventListener('click', (event) => {
      if (!deliberate(event)) return;

      closeChannelMenu();
      run();
    });
    menu.append(item);
  }

  root.addEventListener('pointerdown', () => {
    armed = true;
  });
  sheet.addEventListener('click', (event) => {
    if (deliberate(event)) closeChannelMenu();
  });
  sheet.addEventListener('wheel', () => closeChannelMenu(), { passive: true });

  root.append(sheet, menu);
  document.body.append(element);

  const size = menu.getBoundingClientRect();

  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - size.width - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - size.height - 8))}px`;

  const onKey = (event: KeyboardEvent) => {
    if (event.key === 'Escape') closeChannelMenu();
  };

  document.addEventListener('keydown', onKey, true);

  host = element;
  release = () => document.removeEventListener('keydown', onKey, true);
}

/** Closes Shiver's own channel menu; true when it was open. */
export function closeChannelMenu() {
  const open = !!host;

  release?.();
  release = null;
  host?.remove();
  host = null;

  return open;
}

/** Whether `node` is in Shiver's own channel menu (whose events are retargeted to its host). */
export const isInChannelMenu = (node: Node) => !!host?.contains(node);
