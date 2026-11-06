/** Adapting Sharkord's desktop-shaped client to a finger. */

import { channelPressed, installChannelMenu, requestChannelMenu } from '../../shared/web/bridge/channel-menu';
import { ensureStyle } from '../../shared/web/bridge/dom';
import {
  CHANNEL_ITEM,
  DM_ITEM,
  DM_TOGGLE,
  isDmRowFor,
  markAllChannelsRead,
  MESSAGE_ITEM,
  SERVER_VIEW,
  SIDEBAR
} from '../../shared/web/bridge/sharkord';

/** a press held this long is a hold */
export const HOLD_MS = 500;
/** how far a finger may drift and still be holding still */
export const HOLD_SLOP = 10;
/** how long after a hold its own click may still arrive and must be ignored */
const CLICK_GRACE_MS = 700;
/** Tailwind's `md`, where Sharkord stops hiding its sidebar */
const WIDE_LAYOUT = 768;
/** the scrim beside Sharkord's open drawer, which closes it when pressed */
const DRAWER_SCRIM = `${SERVER_VIEW} [class~="md:hidden"][class~="inset-0"][class~="z-30"]`;
/** how long after the row is pressed the drawer is closed, so the press lands first */
const CLOSE_AFTER_MS = 120;
const MESSAGE_ACTIONS_CLASS = 'shiver-message-actions';
const APPEAR_TIMEOUT_MS = 15_000;

/**
 * Page styles for touch:
 * - a held message shows Sharkord's own hover toolbar (reply, react, edit...); messages are
 *   unselectable until then, so the first hold opens the toolbar instead of starting a selection;
 * - the composer's drag-to-resize divider is inert (a thumb brushing it pinned the height), and its
 *   height is capped at Sharkord's own 35vh; its scrollbar no longer covers the send button;
 * - the "Logging in automatically" text on the loading screen is hidden (the spinner stays).
 */
export function installTouchStyles() {
  const divider = '[role="separator"][aria-label="Resize chat input"], [role="separator"].cursor-row-resize';

  ensureStyle('shiver-touch-style').textContent = `
.${MESSAGE_ACTIONS_CLASS} [class*="group-hover:flex"] { display: flex !important; }
${MESSAGE_ITEM} { -webkit-user-select: none; user-select: none; -webkit-touch-callout: none; }
.${MESSAGE_ACTIONS_CLASS}, ${MESSAGE_ITEM} input, ${MESSAGE_ITEM} textarea, ${MESSAGE_ITEM} [contenteditable] {
  -webkit-user-select: text; user-select: text; }
${divider} { pointer-events: none !important; cursor: default !important; }
.compose-container { height: auto !important; max-height: 35vh; }
.compose-scroll-row { scrollbar-width: none; }
.compose-scroll-row::-webkit-scrollbar { width: 0; height: 0; display: none; }
div.flex.flex-col.justify-center.items-center.h-full.gap-2 > span.text-xl { display: none !important; }
`;

  // a height pinned by an earlier drag would be restored from storage
  for (const key of ['sharkord-chat-input-height-vh', 'sharkord-thread-input-height-vh']) {
    try {
      localStorage.removeItem(key);
    } catch {
      // storage blocked
    }
  }
}

function clearMessageActions() {
  for (const shown of document.querySelectorAll(`.${MESSAGE_ACTIONS_CLASS}`)) shown.classList.remove(MESSAGE_ACTIONS_CLASS);
}

/**
 * Long presses on channels and messages. On a message it reveals Sharkord's toolbar; on a channel it
 * asks for Shiver's item in the channel menu Sharkord opens for the hold (`channel-menu.ts`). The
 * press's own click is swallowed.
 */
export function installLongPress() {
  let timer = 0;
  let startX = 0;
  let startY = 0;
  let row: HTMLElement | null = null;
  /** when the last hold fired (a time, so a stale value cannot swallow a later tap) */
  let heldAt = 0;

  const cancel = () => {
    window.clearTimeout(timer);
    timer = 0;
    row = null;
  };

  installChannelMenu({
    markAllRead: markAllChannelsRead,
    touch: true,
    // Sharkord's menu can open before the hold fires, and is then what the press opened
    onJoined: () => {
      heldAt = Date.now();
      cancel();
    }
  });

  document.addEventListener(
    'touchstart',
    (event) => {
      const within = event.target instanceof Element ? event.target : null;
      const target = within?.closest(`${CHANNEL_ITEM}, ${MESSAGE_ITEM}`);
      const touch = event.touches[0];

      cancel();

      // a touch in the revealed toolbar, or in what it opened (portalled), is using it
      if (!within?.closest(`.${MESSAGE_ACTIONS_CLASS}, [role="menu"], [role="dialog"], [role="listbox"], [data-radix-popper-content-wrapper]`)) {
        clearMessageActions();
      }

      if (!touch || !(target instanceof HTMLElement)) return;

      row = target;
      startX = touch.clientX;
      startY = touch.clientY;

      // a menu Sharkord opens for a held message is not a channel's
      channelPressed(target.matches(CHANNEL_ITEM) ? target : null);

      timer = window.setTimeout(() => {
        const held = row;

        cancel();

        if (!held) return;

        heldAt = Date.now();

        if (held.matches(MESSAGE_ITEM)) held.classList.add(MESSAGE_ACTIONS_CLASS);
        else requestChannelMenu(held);
      }, HOLD_MS);
    },
    { passive: true, capture: true }
  );

  document.addEventListener(
    'touchmove',
    (event) => {
      const touch = event.touches[0];

      if (touch && (Math.abs(touch.clientX - startX) > HOLD_SLOP || Math.abs(touch.clientY - startY) > HOLD_SLOP)) cancel();
    },
    { passive: true, capture: true }
  );
  document.addEventListener('touchend', cancel, { passive: true, capture: true });
  document.addEventListener('touchcancel', cancel, { passive: true, capture: true });

  document.addEventListener(
    'click',
    (event) => {
      if (!heldAt) return;

      const target = event.target instanceof Node ? event.target : null;

      // taps inside Sharkord's menu are its own
      if (target instanceof Element && target.closest('[role="menu"]')) return;

      const wasThePress = Date.now() - heldAt < CLICK_GRACE_MS;

      heldAt = 0;

      if (wasThePress) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
    true
  );
}

/**
 * Makes the phone keyboard's return key insert a line break in the message composer (Sharkord
 * sends on Enter and breaks on Shift+Enter, and a phone has no shift to hold) by re-dispatching it
 * with `shiftKey`. Other fields, and the mention/emoji suggestion popovers, keep Enter, and so does
 * Shiver's own (untrusted) Enter, which is how Send saves an edit (`edit.ts`).
 */
export function installReturnMakesALine() {
  window.addEventListener(
    'keydown',
    (event) => {
      if (event.key !== 'Enter' || event.shiftKey || !event.isTrusted) return;

      const target = event.target as HTMLElement | null;

      if (!target?.closest?.('.ProseMirror') || document.querySelector('.bg-popover')) return;

      event.preventDefault();
      event.stopImmediatePropagation();
      target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', shiftKey: true, bubbles: true, cancelable: true }));
    },
    true
  );
}

/** Calls `use` with what `find` finds, now or once it appears (for a while). */
function whenFound(find: () => HTMLElement | null | undefined, use: (element: HTMLElement) => void) {
  const existing = find();

  if (existing) return use(existing);

  const observer = new MutationObserver(() => {
    const found = find();

    if (!found) return;

    observer.disconnect();
    use(found);
  });

  observer.observe(document.body, { childList: true, subtree: true });
  window.setTimeout(() => observer.disconnect(), APPEAR_TIMEOUT_MS);
}

/** Calls `use` with the first element matching `selector`, now or once it appears (for a while). */
function whenPresent(selector: string, use: (element: HTMLElement) => void) {
  whenFound(() => {
    const found = document.querySelector(selector);

    return found instanceof HTMLElement ? found : null;
  }, use);
}

/** Whether Sharkord's drawer is on screen (always, above `md`). */
export function drawerIsOpen() {
  const sidebar = document.querySelector(SIDEBAR);

  return sidebar instanceof HTMLElement ? sidebar.getBoundingClientRect().right > 1 : window.innerWidth >= WIDE_LAYOUT;
}

/** Opens Sharkord's drawer by performing the swipe it listens for (it has no API). */
function openDrawer() {
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

/** Closes Sharkord's drawer, as pressing beside it does. */
function closeDrawer() {
  document.querySelector<HTMLElement>(DRAWER_SCRIM)?.click();
}

/** Opens the drawer and Sharkord's DM list, once the client has rendered its toggle. */
function openDirectMessages() {
  whenPresent(DM_TOGGLE, (toggle) => {
    openDrawer();
    // after the drawer's transition
    window.setTimeout(() => toggle.click(), 80);
  });
}

/**
 * Opens the DM with `userName`, and closes the drawer over it. Rows carry no id, so the row is the
 * one showing exactly that name (as on desktop), waited for until it renders: the first rows may not
 * be it.
 */
export function openConversation(userName: string) {
  openDirectMessages();

  whenFound(
    () => [...document.querySelectorAll<HTMLElement>(DM_ITEM)].find((row) => isDmRowFor(row, userName)),
    (row) => {
      row.click();
      window.setTimeout(closeDrawer, CLOSE_AFTER_MS);
    }
  );
}
