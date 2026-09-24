/**
 * Runs at document start in every page of the webview (Shiver's own and each server's), before
 * the page's own scripts. Keep it small: it runs on every navigation.
 *
 * 1. Takes a `#shiver-seed=<key>.<token>` Shiver navigated with out of the URL and installs the session
 *    shim, so the server's client signs in without the token ever being written to storage. The key
 *    must be SHA-256 of this script's secret and the page's origin, so a link cannot seed a session
 *    and a server that reads its own key learns nothing usable on another. Sharkord auto-logs in
 *    only after its plugins load, well after the digest.
 * 2. Applies Sharkord's light/dark class before first paint (its own effect runs only after React
 *    mounts, so its light-first stylesheet would flash white on every server switch). Same rule as
 *    Sharkord's `ThemeProvider`: the stored `vite-ui-theme`, default dark, `system` follows the OS.
 */

import { installSessionShim, takeSeedFromLocation } from '../../shared/web/session';

declare const SHIVER_SEED_KEY: string;

const hex = (digest: ArrayBuffer) => Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');

try {
  const seed = takeSeedFromLocation();

  if (seed)
    crypto.subtle
      .digest('SHA-256', new TextEncoder().encode(SHIVER_SEED_KEY + location.origin))
      .then((digest) => {
        if (hex(digest) === seed[0] && !window.__SHIVER_SESSION_SHIM__) installSessionShim(seed[1]);
      })
      .catch(() => {});
} catch {
  // never take a page down; without the shim the bridge falls back to seeding storage
}

const chosenTheme = () => {
  let stored: string | null = null;

  try {
    stored = localStorage.getItem('vite-ui-theme');
  } catch {
    // storage refused: the default applies
  }

  const theme = stored === 'dark' || stored === 'light' || stored === 'system' ? stored : 'dark';

  if (theme !== 'system') return theme;

  try {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  } catch {
    return 'dark';
  }
};

/** Adds the class once `<html>` exists; true when done (or already decided). */
const applyTheme = () => {
  const root = document.documentElement;

  if (!root) return false;

  if (!root.classList.contains('dark') && !root.classList.contains('light')) root.classList.add(chosenTheme());

  return true;
};

try {
  if (!applyTheme()) {
    const observer = new MutationObserver(() => {
      if (applyTheme()) observer.disconnect();
    });

    observer.observe(document, { childList: true });
  }
} catch {
  // the cost of failing is one white frame
}

export {};
