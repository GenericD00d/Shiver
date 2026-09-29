/**
 * Asking for the rail, and leaving for Shiver's own page. The page has no IPC and is told nothing
 * about the user's other servers, so both are plain navigations to Shiver's page: `#home` asks for
 * the rail, which the core draws over this page instead (`rail.rs`), so the page stays loaded;
 * any other fragment (`#open=`) leaves.
 */

import { IMAGE_VIEWER, SIDEBAR } from '../../shared/web/bridge/sharkord';
import { drawerIsOpen } from './touch';

/** Sharkord's own swipe threshold */
const SWIPE_THRESHOLD = 50;

let home = '';

export function setHome(url: string) {
  home = url.replace(/#.*$/, '');
}

/** Navigates to Shiver's own page, which acts on `hash`; true when it went. */
export function goHome(hash: string) {
  if (home) window.location.assign(home + hash);

  return !!home;
}

/** Asks for the rail (drawn over this page); true when asked. */
export const openRail = () => goHome('#home');

/**
 * A right swipe past Sharkord's open drawer (starting over it) opens the rail; a closed drawer's
 * swipe stays Sharkord's. On a page without a sidebar (sign-in, errors) any right swipe opens it.
 * A finger on the full-screen picture is panning it, never leaving.
 */
export function installRailSwipe() {
  let startX = 0;
  let startY = 0;
  let lastX = 0;
  let lastY = 0;
  let onPicture = false;
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
      onPicture = event.target instanceof Element && !!event.target.closest(IMAGE_VIEWER);
    }
  };

  window.addEventListener('touchstart', (event) => track(event, true), passive);
  window.addEventListener('touchmove', (event) => track(event, false), passive);
  window.addEventListener(
    'touchend',
    (event) => {
      const dx = lastX - startX;

      if (!event.isTrusted || onPicture || dx < SWIPE_THRESHOLD || dx <= Math.abs(lastY - startY)) return;

      const sidebar = document.querySelector(SIDEBAR);

      if (sidebar instanceof HTMLElement && !(drawerIsOpen() && startX <= Math.max(sidebar.getBoundingClientRect().right, 0) + 40)) return;

      event.stopPropagation();
      openRail();
    },
    { capture: true }
  );
}
