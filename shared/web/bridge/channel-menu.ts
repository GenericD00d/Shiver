/**
 * Shiver's item in Sharkord's channel menu, opened by a right-click (desktop) or a long press
 * (mobile) on a sidebar channel: mark every channel read. Since 0.0.27 Sharkord gives everyone that
 * menu, with its own mute item (which Shiver's mutes follow, `mutes.ts`); an older server gives it
 * only to channel managers, and everyone else gets no menu.
 *
 * The bridges own the gesture and report it: `channelPressed` when a press begins, and
 * `requestChannelMenu` when it asks for a menu.
 */

import { ensureStyle } from './dom';
import { channelOfRow, pressEscape } from './sharkord';

export type ChannelMenuSetup = {
  markAllRead: () => void;
  /** a longer wait for Sharkord's menu, which a long press opens late */
  touch: boolean;
  /** told when Sharkord's menu took Shiver's item, which may be before the gesture asked */
  onJoined?: () => void;
};

/** How long after a press a newly added menu is taken to belong to it. */
const PRESS_WINDOW_MS = 1500;
/** Shiver's item in Sharkord's menu */
const ITEM_CLASS = 'shiver-menu-item';

let setup: ChannelMenuSetup | null = null;
/** the row the last press began on, and when */
let press: { row: Element; at: number } | null = null;
let waiting = 0;

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

    if (!menu || !channelOfRow(press.row)) return;

    join(menu);
    forgetPress();
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
 * (null) somewhere else, which forgets the last press.
 */
export function channelPressed(row: Element | null) {
  forgetPress();

  if (row) press = { row, at: Date.now() };
}

/**
 * The press on `row` asks for a menu: Sharkord's gets Shiver's item if it is open by the end of the
 * grace. False when the row is no channel Shiver knows.
 */
export function requestChannelMenu(row: Element) {
  if (!setup || !channelOfRow(row)) return false;

  window.clearTimeout(waiting);
  waiting = window.setTimeout(
    () => {
      forgetPress();

      // Radix moves an already-open menu rather than adding one, so the observer never sees it
      const open = openMenuOnScreen();

      if (open) join(open);
    },
    // a right-click opens Sharkord's menu at once; a long press opens it after Shiver's hold
    setup.touch ? 450 : 250
  );

  return true;
}

/**
 * Adds Shiver's item to one of Sharkord's own menus, styled like its siblings. Replaces one left from
 * an earlier open, since Radix reuses its menu.
 */
function join(menu: HTMLElement) {
  if (!setup) return;

  for (const stale of menu.querySelectorAll(`.${ITEM_CLASS}`)) stale.remove();

  addItem(menu, 'Mark all as read', setup.markAllRead);
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

/** Closes Sharkord's open menu (a channel's, or any other), as Escape would; true when one was open. */
export function closeOpenMenu() {
  if (!openMenuOnScreen()) return false;

  pressEscape();

  return true;
}
