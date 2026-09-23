/** Adapting Sharkord's desktop-shaped client to a finger. */

import { addedMenu, ensureStyle, openMenuOnScreen } from '../../shared/web/bridge/dom';
import { addMuteItem, CHANNEL_ITEM, channelOfRow, DM_ITEM, DM_TOGGLE, MESSAGE_ITEM, type SharkordChannel } from '../../shared/web/bridge/sharkord';
import { CLICK_GRACE_MS, HOLD_MS, HOLD_SLOP, openDrawer } from './rail';

type Mutes = { has: (channelId: number) => boolean; toggle: (channelId: number) => void };

const MESSAGE_ACTIONS_CLASS = 'shiver-message-actions';
/** how long after a press Sharkord's own menu (it has one for channel managers) may still arrive */
const SHARKORD_MENU_GRACE_MS = 450;
const SHARKORD_MENU_WINDOW_MS = 1500;
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

let menuHost: HTMLElement | null = null;

/** Closes Shiver's channel menu; true when one was open. */
export function closeChannelMenu() {
  const open = !!menuHost;

  menuHost?.remove();
  menuHost = null;

  return open;
}

function clearMessageActions() {
  for (const shown of document.querySelectorAll(`.${MESSAGE_ACTIONS_CLASS}`)) shown.classList.remove(MESSAGE_ACTIONS_CLASS);
}

/**
 * Long presses on channels and messages. On a message it reveals Sharkord's toolbar. On a channel
 * it adds "Mute in Shiver" to Sharkord's own menu when one appears (for channel managers), or
 * after a grace opens Shiver's own mute menu. The press's own click is swallowed.
 */
export function installChannelMenu(mutes: Mutes) {
  let timer = 0;
  let fallback = 0;
  let startX = 0;
  let startY = 0;
  let row: HTMLElement | null = null;
  /** when the last hold fired (a time, so a stale value cannot swallow a later tap) */
  let heldAt = 0;
  let pending: SharkordChannel | null = null;
  /** the row a finger went down on: Sharkord's menu can arrive before Shiver's hold fires */
  let pressedRow: HTMLElement | null = null;
  let pressedAt = 0;

  const cancel = () => {
    window.clearTimeout(timer);
    timer = 0;
    row = null;
  };

  const settle = () => {
    window.clearTimeout(fallback);
    fallback = 0;
    pending = null;
    pressedRow = null;
    pressedAt = 0;
  };

  const join = (menu: HTMLElement, channel: SharkordChannel) => addMuteItem(menu, mutes.has(channel.id), () => mutes.toggle(channel.id));

  new MutationObserver((records) => {
    if (!pressedRow || Date.now() - pressedAt > SHARKORD_MENU_WINDOW_MS) return;

    const menu = addedMenu(records);
    const channel = menu ? (pending ?? channelOfRow(pressedRow)) : null;

    if (!menu || !channel) return;

    join(menu, channel);
    heldAt = Date.now();
    cancel();
    closeChannelMenu();
    settle();
  }).observe(document.body, { childList: true, subtree: true });

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

      row = pressedRow = target;
      pressedAt = Date.now();
      startX = touch.clientX;
      startY = touch.clientY;

      timer = window.setTimeout(() => {
        const held = row;

        cancel();

        if (!held) return;

        heldAt = Date.now();

        if (held.matches(MESSAGE_ITEM)) {
          held.classList.add(MESSAGE_ACTIONS_CLASS);

          return;
        }

        pending = channelOfRow(held);

        if (!pending) return;

        fallback = window.setTimeout(() => {
          const channel = pending;

          settle();

          if (!channel) return;

          // Radix moves an already-open menu rather than adding one
          const open = openMenuOnScreen();

          if (open) join(open, channel);
          else openChannelMenu(channel, startX, startY, mutes);
        }, SHARKORD_MENU_GRACE_MS);
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

      // taps inside Shiver's menu (retargeted to its host) or Sharkord's menu are theirs
      if (target && (menuHost?.contains(target) || (target instanceof Element && target.closest('[role="menu"]')))) return;

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

/** Shiver's one-item mute menu, in a closed shadow root over a sheet that closes it. */
function openChannelMenu(channel: SharkordChannel, x: number, y: number, mutes: Mutes) {
  closeChannelMenu();

  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'closed' });
  const sheet = document.createElement('div');
  const menu = document.createElement('div');
  const item = document.createElement('button');
  const openedAt = Date.now();

  host.id = 'shiver-channel-menu';
  root.innerHTML = `<style>
:host { position: fixed; inset: 0; z-index: 2147483646; }
.sheet { position: absolute; inset: 0; }
.menu { position: absolute; min-width: 200px; max-width: 70vw; padding: 6px; border-radius: 12px;
  border: 1px solid rgb(255 255 255 / 12%); background: #1f1f1f; color: #fafafa; box-shadow: 0 12px 32px rgb(0 0 0 / 55%);
  font: 500 14px/1.2 system-ui, -apple-system, "Segoe UI", sans-serif; }
.menu-item { display: block; width: 100%; padding: 11px 12px; border: none; border-radius: 8px;
  background: none; color: inherit; font: inherit; text-align: left; }
.menu-item:active { background: var(--shiver-surface-hover, #333333); }
</style>`;

  item.className = 'menu-item';
  item.type = 'button';
  item.textContent = `${mutes.has(channel.id) ? 'Unmute' : 'Mute'} #${channel.name}`;
  item.addEventListener('click', () => {
    mutes.toggle(channel.id);
    closeChannelMenu();
  });

  menu.className = 'menu';
  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - 220))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - 80))}px`;
  menu.append(item);

  // the finger that opened it lifts over the sheet, so that first tap is ignored
  sheet.className = 'sheet';
  sheet.addEventListener('click', () => {
    if (Date.now() - openedAt >= CLICK_GRACE_MS) closeChannelMenu();
  });

  root.append(sheet, menu);
  document.body.append(host);
  menuHost = host;
}

/**
 * Makes the phone keyboard's return key insert a line break in the message composer (Sharkord
 * sends on Enter and breaks on Shift+Enter, and a phone has no shift to hold) by re-dispatching it
 * with `shiftKey`. Other fields, and the mention/emoji suggestion popovers, keep Enter.
 */
export function installReturnMakesALine() {
  window.addEventListener(
    'keydown',
    (event) => {
      if (event.key !== 'Enter' || event.shiftKey) return;

      const target = event.target as HTMLElement | null;

      if (!target?.closest?.('.ProseMirror') || document.querySelector('.bg-popover')) return;

      event.preventDefault();
      event.stopImmediatePropagation();
      target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', shiftKey: true, bubbles: true, cancelable: true }));
    },
    true
  );
}

/** Sharkord's reaction pill (`h-9 gap-1`; the border class only marks your own). */
const REACTION_PILL = 'button[class~="h-9"][class~="gap-1"]';

/**
 * Long-press a reaction to see who reacted. Sharkord's Radix tooltip already lists them but
 * refuses touch pointers, so the hold replays the pointer events as a mouse and Sharkord opens its
 * own tooltip. The hold's click is swallowed so it does not toggle your reaction.
 */
export function installReactionNames() {
  let timer = 0;
  let shown: HTMLElement | null = null;
  let startX = 0;
  let startY = 0;
  const options = { passive: true, capture: true } as const;

  const tell = (pill: HTMLElement, entering: boolean) => {
    for (const name of entering ? ['pointerenter', 'pointermove'] : ['pointerleave']) {
      pill.dispatchEvent(new PointerEvent(name, { bubbles: name === 'pointermove', pointerType: 'mouse' }));
    }
  };

  const cancel = () => {
    window.clearTimeout(timer);
    timer = 0;
  };

  document.addEventListener(
    'touchstart',
    (event) => {
      const touch = event.touches[0];

      if (!touch) return;

      const pill = (event.target as HTMLElement | null)?.closest<HTMLElement>(REACTION_PILL) ?? null;

      if (shown) tell(shown, false);

      shown = null;

      if (!pill) return;

      startX = touch.clientX;
      startY = touch.clientY;
      timer = window.setTimeout(() => {
        shown = pill;
        tell(pill, true);
        pill.dataset.shiverHeld = '1';
      }, HOLD_MS);
    },
    options
  );
  document.addEventListener(
    'touchmove',
    (event) => {
      const touch = event.touches[0];

      if (touch && (Math.abs(touch.clientX - startX) > HOLD_SLOP || Math.abs(touch.clientY - startY) > HOLD_SLOP)) cancel();
    },
    options
  );
  document.addEventListener(
    'touchend',
    () => {
      cancel();

      // lifting the finger sends a `pointerleave` Radix acts on, so the tooltip is reopened
      if (shown) {
        window.setTimeout(() => {
          if (shown) tell(shown, true);
        }, 60);
      }
    },
    options
  );
  document.addEventListener('touchcancel', cancel, options);
  document.addEventListener(
    'click',
    (event) => {
      const pill = (event.target as HTMLElement | null)?.closest<HTMLElement>(REACTION_PILL);

      if (!pill?.dataset.shiverHeld) return;

      delete pill.dataset.shiverHeld;
      event.preventDefault();
      event.stopPropagation();
    },
    true
  );
}

/** Calls `use` with the first element matching `selector`, now or once it appears (for a while). */
function whenPresent(selector: string, use: (element: HTMLElement) => void) {
  const existing = document.querySelector(selector);

  if (existing instanceof HTMLElement) return use(existing);

  const observer = new MutationObserver(() => {
    const found = document.querySelector(selector);

    if (!(found instanceof HTMLElement)) return;

    observer.disconnect();
    use(found);
  });

  observer.observe(document.body, { childList: true, subtree: true });
  window.setTimeout(() => observer.disconnect(), APPEAR_TIMEOUT_MS);
}

/** Opens the drawer and Sharkord's DM list, once the client has rendered its toggle. */
export function openDirectMessages() {
  whenPresent(DM_TOGGLE, (toggle) => {
    openDrawer();
    // after the drawer's transition
    window.setTimeout(() => toggle.click(), 80);
  });
}

/**
 * Opens the DM with `userName`. Rows carry no id; the text may start with an avatar initial when
 * there is no picture ("WWanderer"), so up to two leading characters are allowed.
 */
export function openConversation(userName: string) {
  openDirectMessages();

  whenPresent(DM_ITEM, () => {
    for (const row of document.querySelectorAll<HTMLElement>(DM_ITEM)) {
      const text = row.textContent?.trim() ?? '';

      if (text === userName || (text.endsWith(userName) && text.length - userName.length <= 2)) {
        row.click();

        return;
      }
    }
  });
}
