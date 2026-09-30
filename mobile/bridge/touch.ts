/** Adapting Sharkord's desktop-shaped client to a finger. */

import {
  channelPressed,
  type ChannelMutes,
  installChannelMenu,
  isInChannelMenu,
  requestChannelMenu
} from '../../shared/web/bridge/channel-menu';
import { ensureStyle } from '../../shared/web/bridge/dom';
import {
  CHANNEL_ITEM,
  DM_ITEM,
  DM_TOGGLE,
  IMAGE_VIEWER,
  markAllChannelsRead,
  MESSAGE_ITEM,
  SERVER_VIEW,
  SIDEBAR
} from '../../shared/web/bridge/sharkord';

/** a press held this long is a hold */
const HOLD_MS = 500;
/** how far a finger may drift and still be holding still */
const HOLD_SLOP = 10;
/** how long after a hold its own click may still arrive and must be ignored */
const CLICK_GRACE_MS = 700;
/** Tailwind's `md`, where Sharkord stops hiding its sidebar */
const WIDE_LAYOUT = 768;
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
 * asks for the channel menu (`channel-menu.ts`). The press's own click is swallowed.
 */
export function installLongPress(mutes: ChannelMutes) {
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
    mutes,
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
        else requestChannelMenu(held, startX, startY);
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

      // taps inside Shiver's menu or Sharkord's are theirs
      if (target && (isInChannelMenu(target) || (target instanceof Element && target.closest('[role="menu"]')))) return;

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

const MAX_ZOOM = 6;
/** a second tap this soon (and this close) after the first is a double tap */
const DOUBLE_TAP_MS = 300;
const DOUBLE_TAP_ZOOM = 2.5;

/**
 * Sharkord's full-screen picture zooms with the mouse wheel and pans by mouse drag only. Here it
 * pinches, pans with a finger once zoomed, and double-taps in and out. The viewer's touches go no
 * further, since they would otherwise reach Sharkord's swipes (through the portal) and open the
 * drawer behind it.
 */
export function installImageZoom() {
  ensureStyle('shiver-image-zoom').textContent = `${IMAGE_VIEWER} img { touch-action: none; }`;

  type Point = { x: number; y: number };

  let image: HTMLImageElement | null = null;
  /** Sharkord's opening size, which is also the smallest */
  let base = 1;
  let scale = 1;
  let shift: Point = { x: 0, y: 0 };
  /** the gesture in progress, from where it started */
  let start: { points: Point[]; scale: number; shift: Point; centre: Point; at: number } | null = null;
  let lastTap: { at: number; point: Point } | null = null;

  const pointsOf = (touches: TouchList) => [...touches].map((touch) => ({ x: touch.clientX, y: touch.clientY }));
  const middle = (points: Point[]) => ({
    x: points.reduce((sum, point) => sum + point.x, 0) / points.length,
    y: points.reduce((sum, point) => sum + point.y, 0) / points.length
  });
  const apart = ([a, b]: Point[]) => Math.hypot(a.x - b.x, a.y - b.y);

  const apply = () => {
    if (image) image.style.transform = `translate(${shift.x}px, ${shift.y}px) scale(${scale})`;
  };

  /** Scales to `next`, keeping what was under `from` (at the gesture's start) under `to`. */
  const zoomAbout = (next: number, from: Point, to: Point) => {
    if (!start) return;

    const factor = next / start.scale;

    scale = next;
    shift = {
      x: to.x - start.centre.x - (from.x - start.centre.x - start.shift.x) * factor,
      y: to.y - start.centre.y - (from.y - start.centre.y - start.shift.y) * factor
    };
  };

  const begin = (touches: TouchList) => {
    if (!image) return;

    const rect = image.getBoundingClientRect();

    start = {
      points: pointsOf(touches),
      scale,
      shift: { ...shift },
      // the untransformed centre, which scaling is about
      centre: { x: rect.left + rect.width / 2 - shift.x, y: rect.top + rect.height / 2 - shift.y },
      at: Date.now()
    };
  };

  const ownTouch = (event: TouchEvent) => event.target instanceof Element && !!event.target.closest(IMAGE_VIEWER);

  window.addEventListener(
    'touchstart',
    (event) => {
      if (!ownTouch(event)) return;

      event.stopPropagation();

      const target = event.target instanceof HTMLImageElement ? event.target : null;

      if (!target) return;

      if (target !== image) {
        image = target;
        base = Number(/scale\(([\d.]+)\)/.exec(target.style.transform)?.[1]) || 1;
        scale = base;
        shift = { x: 0, y: 0 };
      }

      begin(event.touches);
    },
    { capture: true, passive: true }
  );

  window.addEventListener(
    'touchmove',
    (event) => {
      if (!ownTouch(event)) return;

      event.stopPropagation();

      if (!start || !image) return;

      const points = pointsOf(event.touches);

      if (points.length >= 2 && start.points.length >= 2) {
        const next = Math.min(Math.max((start.scale * apart(points)) / apart(start.points), base * 0.5), MAX_ZOOM);

        zoomAbout(next, middle(start.points), middle(points));
      } else if (points.length === 1 && scale > base) {
        shift = { x: start.shift.x + points[0].x - start.points[0].x, y: start.shift.y + points[0].y - start.points[0].y };
      }

      apply();
    },
    { capture: true, passive: true }
  );

  const end = (event: TouchEvent) => {
    if (!ownTouch(event)) return;

    event.stopPropagation();

    if (!start || !image) return;

    const tapped = event.touches.length === 0 && start.points.length === 1 && Date.now() - start.at < DOUBLE_TAP_MS;
    const point = start.points[0];

    if (tapped && lastTap && Date.now() - lastTap.at < DOUBLE_TAP_MS && Math.hypot(point.x - lastTap.point.x, point.y - lastTap.point.y) < 40) {
      lastTap = null;

      if (scale > base) {
        scale = base;
        shift = { x: 0, y: 0 };
      } else {
        zoomAbout(DOUBLE_TAP_ZOOM, point, point);
      }
    } else if (tapped) {
      lastTap = { at: Date.now(), point };
    }

    // a pinch let out below the opening size springs back to it
    if (scale < base) {
      scale = base;
      shift = { x: 0, y: 0 };
    }

    apply();

    // a finger still down carries on as a pan from here
    if (event.touches.length) begin(event.touches);
    else start = null;
  };

  window.addEventListener('touchend', end, { capture: true, passive: true });
  window.addEventListener('touchcancel', end, { capture: true, passive: true });
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

/** Opens the drawer and Sharkord's DM list, once the client has rendered its toggle. */
function openDirectMessages() {
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
