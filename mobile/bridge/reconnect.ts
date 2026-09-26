/**
 * Recovering the client after Android has had the app asleep. Sharkord's own reconnect timers do
 * not run in a backgrounded webview, so the user returns to a client that gave up. Shiver presses
 * Sharkord's own "try again" button (absent when reconnecting is not allowed, e.g. a ban), after
 * re-seeding the session it holds, or has Shiver reopen the server once it answers again. A bounded
 * number of attempts, counted across reloads in `sessionStorage`; coming back to the app resets them,
 * and they lapse a while after the last.
 *
 * Either way the user sees only a small spinner in the accent colour, never a wall of text.
 */

import { ensureStyle, onDomSettled } from '../../shared/web/bridge/dom';
import { COMPOSE_EDITOR, RECONNECTING_OVERLAY, SIDEBAR } from '../../shared/web/bridge/sharkord';
import { installSessionShim } from '../../shared/web/session';
import { goHome } from './home';

const POLL_MS = 2000;
/**
 * Waits before each attempt. The last lands past a minute: Sharkord allows five joins a minute, and
 * a client refused for joining too often looks exactly like one whose session was refused.
 */
const DELAYS_MS = [0, 3000, 15000, 65000];
const ATTEMPTS_KEY = 'shiver-reconnect-attempts';
/** A spent budget is forgotten this long after its last attempt, so opening the server later tries again. */
const BUDGET_MS = 90_000;

const readAttempts = () => {
  try {
    const [attempts, at] = (sessionStorage.getItem(ATTEMPTS_KEY) ?? '').split(':').map(Number);

    return Date.now() - at < BUDGET_MS ? attempts || 0 : 0;
  } catch {
    return 0;
  }
};

const writeAttempts = (attempts: number) => {
  try {
    if (attempts) sessionStorage.setItem(ATTEMPTS_KEY, `${attempts}:${Date.now()}`);
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

export function installAutoReconnect(session: string | null, entryId: string) {
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

    showReconnecting();

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

    // nothing on the connect form re-runs auto-login, so Shiver reopens a server that answers
    working = true;
    void reachable().then((ok) => {
      working = false;

      if (ok) goHome(`#open=${encodeURIComponent(entryId)}`);
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

/**
 * Sharkord's own retries (after a dropped connection) put up a dialog that blurs the channel; it is
 * hidden, and a spinner sits just above the chat box while they run, the channel readable behind.
 */
export function installQuietReconnect() {
  ensureStyle('shiver-quiet-reconnect').textContent = `${RECONNECTING_OVERLAY} { display: none !important; }`;

  const update = () => {
    const existing = document.getElementById(RETRYING_ID);

    if (!document.querySelector(RECONNECTING_OVERLAY)) return existing?.remove();

    const chat = document.querySelector(COMPOSE_EDITOR)?.getBoundingClientRect();
    const top = chat && chat.height > 0 ? `${Math.max(chat.top - SPINNER_PX - 12, 0)}px` : `calc(100% - ${SPINNER_PX + 96}px)`;
    const host = existing ?? spinner(RETRYING_ID);

    host.style.top = top;

    if (!existing) document.body.append(host);
  };

  onDomSettled(update);
  window.addEventListener('resize', update);
}

const HOST_ID = 'shiver-reconnect';
const RETRYING_ID = 'shiver-retrying';
const SPINNER_PX = 28;

/** A text-less spinner in the accent colour (Sharkord's own, or the one Shiver themes it with). */
function spinner(id: string) {
  const host = document.createElement('div');

  host.id = id;
  host.style.cssText = `position: fixed; left: calc(50% - ${SPINNER_PX / 2}px); top: calc(50% - ${SPINNER_PX / 2}px); z-index: 2147483646; pointer-events: none;`;
  host.attachShadow({ mode: 'closed' }).innerHTML = `<style>
span { display: block; width: ${SPINNER_PX}px; height: ${SPINNER_PX}px; box-sizing: border-box; border-radius: 50%;
  border: 3px solid rgb(255 255 255 / 14%); border-top-color: var(--primary, #5865f2); animation: spin 900ms linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { span { animation-duration: 3s; } }
</style><span role="progressbar" aria-label="Reconnecting"></span>`;

  return host;
}

/** Covers the server's "connection lost" screen (back and the swipe home still work). */
function showReconnecting() {
  if (document.getElementById(HOST_ID)) return;

  const host = document.createElement('div');

  host.id = HOST_ID;
  host.style.cssText = 'position: fixed; inset: 0; z-index: 2147483645; background: #0a0a0a;';
  host.append(spinner(`${HOST_ID}-spinner`));
  document.body.append(host);
}

function hideReconnecting() {
  document.getElementById(HOST_ID)?.remove();
}
