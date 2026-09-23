/**
 * Recovering the client after Android has had the app asleep. Sharkord's own reconnect timers do
 * not run in a backgrounded webview, so the user returns to a client that gave up. Shiver presses
 * Sharkord's own "try again" button (absent when reconnecting is not allowed, e.g. a ban), after
 * re-seeding the session it holds, or reloads once the server answers again. A bounded number of
 * attempts, counted across reloads in `sessionStorage`; coming back to the app resets them.
 */

import { SIDEBAR } from '../../shared/web/bridge/sharkord';
import { installSessionShim, SEED_PARAM } from '../../shared/web/session';

const POLL_MS = 2000;
const DELAYS_MS = [0, 3000, 8000, 15000];
const ATTEMPTS_KEY = 'shiver-reconnect-attempts';

const readAttempts = () => {
  try {
    return Number(sessionStorage.getItem(ATTEMPTS_KEY)) || 0;
  } catch {
    return 0;
  }
};

const writeAttempts = (attempts: number) => {
  try {
    if (attempts) sessionStorage.setItem(ATTEMPTS_KEY, String(attempts));
    else sessionStorage.removeItem(ATTEMPTS_KEY);
  } catch {
    // in-memory budget only
  }
};

/** Serves `session` to Sharkord's auto-login again (it clears its own after a failed connect). */
const reseed = (session: string) => {
  const shim = window.__SHIVER_SESSION_SHIM__;

  if (shim) shim.seed(session);
  else installSessionShim(session);
};

/** `navigator.onLine` is unreliable on Android, so the server itself is asked. */
const reachable = () =>
  fetch(`${location.origin}/info`, { cache: 'no-store' })
    .then((response) => response.ok)
    .catch(() => false);

export function installAutoReconnect(session: string | null, serverName: string) {
  let attempts = readAttempts();
  let nextAttemptAt = 0;
  /** a probe is in flight */
  let working = false;
  /** when the connect form was first seen (a fresh page shows it briefly before auto-login) */
  let formSince = 0;

  const reset = () => {
    attempts = 0;
    nextAttemptAt = 0;
    formSince = 0;
    writeAttempts(0);
  };

  const tick = () => {
    if (document.hidden || working) return;

    if (document.querySelector(SIDEBAR)) {
      reset();
      hideReconnecting();

      return;
    }

    // Sharkord still counting down its own retries
    if (document.querySelector('[role="alertdialog"]')) return hideReconnecting();

    const button = document.querySelector('button svg[class*="lucide-refresh"]')?.closest('button') ?? null;
    const form = !!document.querySelector('input[type="password"]');

    if (!button && !form) {
      formSince = 0;

      return;
    }

    if (form && !button) {
      formSince ||= Date.now();

      if (Date.now() - formSince < POLL_MS) return;
    }

    // without a session Shiver holds, the connect form is the honest place to be
    if (!session || attempts >= DELAYS_MS.length) return hideReconnecting();

    showReconnecting(serverName);

    nextAttemptAt ||= Date.now() + DELAYS_MS[attempts];

    if (Date.now() < nextAttemptAt) return;

    attempts += 1;
    nextAttemptAt = 0;
    writeAttempts(attempts);

    if (button) {
      reseed(session);
      button.click();

      return;
    }

    // on the connect form nothing re-runs auto-login, so reload, but only into a server that answers;
    // the session travels in the fragment the document-start script reads
    working = true;
    void reachable().then((ok) => {
      working = false;

      if (!ok) return;

      history.replaceState(history.state, '', `${location.pathname}${location.search}#${SEED_PARAM}=${encodeURIComponent(session)}`);
      location.reload();
    });
  };

  window.setInterval(tick, POLL_MS);

  // coming back to the app is the user asking again, which earns a fresh budget
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) return;

    reset();
    tick();
  });

  window.addEventListener('online', tick);
}

const HOST_ID = 'shiver-reconnect';

/** Covers the server's "connection lost" screen (below the rail, which stays usable). */
function showReconnecting(serverName: string) {
  if (document.getElementById(HOST_ID)) return;

  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'closed' });
  const label = document.createElement('p');

  host.id = HOST_ID;
  root.innerHTML = `<style>
:host { position: fixed; inset: 0; z-index: 2147483645; }
div { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 18px; background: #0a0a0a; color: #a1a1a1; font: 500 15px/1.3 system-ui, -apple-system, "Segoe UI", sans-serif; }
span { width: 34px; height: 34px; border-radius: 50%; border: 3px solid rgb(255 255 255 / 14%);
  border-top-color: #e5e5e5; animation: spin 900ms linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { span { animation-duration: 3s; } }
</style><div><span></span></div>`;

  label.textContent = `Reconnecting to ${serverName}…`;
  root.querySelector('div')?.append(label);
  document.body.append(host);
}

function hideReconnecting() {
  document.getElementById(HOST_ID)?.remove();
}
