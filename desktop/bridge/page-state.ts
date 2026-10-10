/** What the core asks about the page: signed out, stalled, ready, fullscreen. */

import {
  CONNECT_FORM,
  RECONNECTING_OVERLAY,
  SERVER_VIEW,
  type SharkordState,
  sharkordStore,
  SIDEBAR
} from '../../shared/web/bridge/sharkord';
import { AUTO_LOGIN } from '../../shared/web/session';
import { hasLucideIcon } from './voice';

// taken before the page's scripts run, so a page cannot fake fullscreen to hide Shiver's chrome
const nativeFullscreenElement = Object.getOwnPropertyDescriptor(Document.prototype, 'fullscreenElement')?.get;

const nativeApply = Reflect.apply;

export const isFullscreen = () => !!nativeFullscreenElement && nativeApply(nativeFullscreenElement, document, []) !== null;

/**
 * Whether the client gave up on the session: the connect form is up and auto-login is no longer
 * `true` (Sharkord sets it false when it refuses a token; absent means nothing was seeded).
 */
export function isSignedOut() {
  if (!document.querySelector(CONNECT_FORM)) return false;

  try {
    return localStorage.getItem(AUTO_LOGIN) !== 'true';
  } catch {
    return true;
  }
}

/** How long a page may go without Sharkord's client starting before it is counted as stalled. */
const STALL_MS = 20_000;

const startedAt = Date.now();

/**
 * Whether the page will not recover by itself, so the core should reload it once the server
 * answers: Sharkord's "connection lost" screen, shown when its own retries (a few seconds' worth)
 * are spent, or no Sharkord client at all (a proxy's error page while the server is down). Not a
 * kick or a ban, whose screens the user should see.
 */
export function isStalled() {
  if (document.querySelector(SIDEBAR) || document.querySelector(RECONNECTING_OVERLAY)) return false;

  // the wifi-off icon is also in the voice controls and Sharkord's retry overlay, both ruled out above
  if (hasLucideIcon(document.documentElement, 'wifi-off')) return true;

  return Date.now() - startedAt > STALL_MS && !sharkordStore();
}

/** Connected (own user known and the server view mounted), or showing its connect form. */
export function isClientReady(state: SharkordState) {
  if (document.querySelector(CONNECT_FORM)) return true;

  return typeof state.ownUserId === 'number' && document.querySelector(SERVER_VIEW) !== null;
}
