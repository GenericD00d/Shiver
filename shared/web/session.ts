/**
 * Keeps a server's session out of the browser profile on disk.
 *
 * Sharkord signs in automatically from `localStorage['sharkord-auto-login-token']` and then keeps
 * the live session in `sessionStorage['sharkord-token']`. Both are files in the webview's profile.
 * This patches `Storage.prototype` so that, while a Shiver-supplied session is active, those keys
 * are served from memory and never written. It steps aside (and storage behaves normally) as soon
 * as Sharkord gives up on the token or the user signs in with "Login automatically" themselves.
 *
 * Must run before the page's own scripts. Idempotent: a second call only changes the session.
 */

const AUTO_LOGIN = 'sharkord-auto-login';
const AUTO_LOGIN_TOKEN = 'sharkord-auto-login-token';
const LIVE_TOKEN = 'sharkord-token';

type Shim = { seed: (token: string | null) => void; active: () => boolean };

declare global {
  interface Window {
    __SHIVER_SESSION_SHIM__?: Shim;
  }
}

export function installSessionShim(token: string | null): Shim {
  const existing = window.__SHIVER_SESSION_SHIM__;

  if (existing) {
    existing.seed(token);

    return existing;
  }

  let seeded: string | null = null;
  let live: string | null = null;

  const proto = Storage.prototype;
  const { getItem, setItem, removeItem } = proto;

  const isLocal = (storage: Storage) => {
    try {
      return storage === window.localStorage;
    } catch {
      return false;
    }
  };

  const isSession = (storage: Storage) => {
    try {
      return storage === window.sessionStorage;
    } catch {
      return false;
    }
  };

  /**
   * Hands control back to real storage. `keepLive` (the user signed in on the page) writes the live
   * session through; otherwise (Sharkord refused the token and removed it) it is dropped.
   */
  const stop = (keepLive: boolean) => {
    const current = live;

    seeded = null;
    live = null;

    if (keepLive && current !== null) {
      try {
        setItem.call(window.sessionStorage, LIVE_TOKEN, current);
      } catch {
        // storage blocked: the page falls back to its connect form
      }
    }
  };

  proto.getItem = function (this: Storage, key: string) {
    if (seeded !== null) {
      if (isLocal(this) && key === AUTO_LOGIN_TOKEN) return seeded;
      if (isLocal(this) && key === AUTO_LOGIN) return 'true';
      if (isSession(this) && key === LIVE_TOKEN) return live;
    }

    return getItem.call(this, key);
  };

  proto.setItem = function (this: Storage, key: string, value: string) {
    if (seeded !== null) {
      if (isSession(this) && key === LIVE_TOKEN) {
        live = String(value);

        return;
      }

      // the user signed in on the page themselves (with or without "Login automatically")
      if (isLocal(this) && (key === AUTO_LOGIN_TOKEN || (key === AUTO_LOGIN && String(value) !== 'true'))) stop(true);
    }

    setItem.call(this, key, value);
  };

  proto.removeItem = function (this: Storage, key: string) {
    if (seeded !== null) {
      if (isSession(this) && key === LIVE_TOKEN) {
        live = null;

        return;
      }

      if (isLocal(this) && key === AUTO_LOGIN_TOKEN) stop(false);
    }

    removeItem.call(this, key);
  };

  const shim: Shim = {
    seed(next) {
      seeded = next || null;
      live = null;

      if (seeded === null) return;

      // a copy an older Shiver (or a crash) left on disk is superseded by the one in memory
      try {
        removeItem.call(window.localStorage, AUTO_LOGIN_TOKEN);
        removeItem.call(window.localStorage, AUTO_LOGIN);
        removeItem.call(window.sessionStorage, LIVE_TOKEN);
      } catch {
        // storage blocked: nothing on disk to clear
      }
    },
    active: () => seeded !== null
  };

  Object.defineProperty(window, '__SHIVER_SESSION_SHIM__', { value: shim, configurable: false, writable: false });

  shim.seed(token);

  return shim;
}

/** Name of the URL fragment parameter that carries a session into a freshly loaded page (mobile). */
export const SEED_PARAM = 'shiver-seed';

/**
 * Takes a `#shiver-seed=<token>` parameter out of the URL (before any page script can see it) and
 * returns the token. Other fragment parameters are left in place.
 */
export function takeSeedFromLocation(): string | null {
  const params = new URLSearchParams(window.location.hash.slice(1));
  const token = params.get(SEED_PARAM);

  if (token === null) return null;

  params.delete(SEED_PARAM);

  const rest = params.toString();

  history.replaceState(history.state, '', `${window.location.pathname}${window.location.search}${rest ? `#${rest}` : ''}`);

  return token || null;
}
