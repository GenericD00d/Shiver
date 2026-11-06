/** Who reacted with what, on a finger: Sharkord's tooltip naming them answers only a mouse. */

import { HOLD_MS, HOLD_SLOP } from './touch';

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
  /** the pill whose hold's click is still to be swallowed */
  let held: HTMLElement | null = null;
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

      // a hold whose finger lifted off the pill never clicked it: its flag must not eat this tap
      if (held) delete held.dataset.shiverHeld;

      held = null;

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
        held = pill;
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
      held = null;
      event.preventDefault();
      event.stopPropagation();
    },
    true
  );
}
