/**
 * Recovering the client after Android has had the app asleep. Sharkord's own reconnect timers do
 * not run in a backgrounded webview, so the user returns to a client that gave up. Shiver presses
 * Sharkord's own "try again" button (absent when reconnecting is not allowed, e.g. a ban), after
 * re-seeding the session it holds, or has Shiver reopen the server once it answers again. A bounded
 * number of attempts, counted across reloads in `sessionStorage`; coming back to the app resets them.
 *
 * Meanwhile the channel stays readable: the page is copied as the app goes to the background, and
 * the copy is shown, frozen, over Sharkord's disconnected screen with a spinner above the chat box.
 */

import { COMPOSE_EDITOR, SIDEBAR } from '../../shared/web/bridge/sharkord';
import { installSessionShim } from '../../shared/web/session';
import { goHome } from './home';

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
      frozen = null;

      return;
    }

    // Sharkord still counting down its own retries, in a dialog over its disconnected screen
    if (document.querySelector('[role="alertdialog"]')) return showReconnecting();

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
    if (document.hidden) return freeze();

    reset();
    tick();
  });

  window.addEventListener('online', tick);
}

const HOST_ID = 'shiver-reconnect';
const SPINNER_PX = 28;

/** The page as it was when the app went to the background, while it showed a channel. */
let frozen: { copy: HTMLElement; scrolls: [Element, number][]; chatTop: number | null } | null = null;

/**
 * Copies what is on screen: every element (Sharkord's CSS still applies), where each scrolled
 * list was, and where the chat box starts. The copy loses its test ids and nested ids (so nothing
 * takes it for the live client), its media and its inputs.
 */
function freeze() {
  frozen = null;

  if (!document.querySelector(SIDEBAR)) return;

  const copy = document.createElement('div');
  const live: Element[] = [];

  for (const child of document.body.children) {
    if (child.id.startsWith('shiver-') || child.tagName === 'SCRIPT') continue;

    live.push(child, ...child.querySelectorAll('*'));
    copy.append(child.cloneNode(true));
  }

  const copied = [...copy.children].flatMap((child) => [child, ...child.querySelectorAll('*')]);
  const scrolls = live.flatMap((element, index): [Element, number][] =>
    element.scrollTop > 0 && copied[index] ? [[copied[index], element.scrollTop]] : []
  );

  for (const element of copy.querySelectorAll('[data-testid]')) element.removeAttribute('data-testid');
  // ids stay only where layout CSS may name them (`#root`); a lookup never finds the copy first
  for (const element of copy.querySelectorAll(':scope > * [id]')) element.removeAttribute('id');
  for (const element of copy.querySelectorAll('video, audio, iframe, object, embed')) element.remove();
  for (const element of copy.querySelectorAll('[contenteditable]')) element.setAttribute('contenteditable', 'false');
  for (const element of copy.querySelectorAll('input, textarea, select, button')) element.setAttribute('disabled', '');

  const chat = document.querySelector(COMPOSE_EDITOR)?.getBoundingClientRect();

  frozen = { copy, scrolls, chatTop: chat && chat.height > 0 ? chat.top : null };
}

/** Covers Sharkord's disconnected screen with the frozen page (or its background) and a small spinner in the accent colour. */
function showReconnecting() {
  if (document.getElementById(HOST_ID)) return;

  const host = document.createElement('div');
  const spinner = document.createElement('div');
  const top = frozen?.chatTop != null ? `${Math.max(frozen.chatTop - SPINNER_PX - 12, 0)}px` : `calc(100% - ${SPINNER_PX + 96}px)`;

  host.id = HOST_ID;
  host.style.cssText = `position: fixed; inset: 0; z-index: 2147483645; overflow: hidden; background: ${getComputedStyle(document.body).backgroundColor || '#0a0a0a'};`;
  // the copy is there to read: its taps do nothing
  host.addEventListener(
    'click',
    (event) => {
      event.preventDefault();
      event.stopPropagation();
    },
    true
  );

  spinner.attachShadow({ mode: 'closed' }).innerHTML = `<style>
:host { position: fixed; left: calc(50% - ${SPINNER_PX / 2}px); top: ${top}; z-index: 1; pointer-events: none; }
span { display: block; width: ${SPINNER_PX}px; height: ${SPINNER_PX}px; box-sizing: border-box; border-radius: 50%;
  border: 3px solid rgb(255 255 255 / 14%); border-top-color: var(--primary, #5865f2); animation: spin 900ms linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { span { animation-duration: 3s; } }
</style><span role="progressbar" aria-label="Reconnecting"></span>`;

  if (frozen) host.append(frozen.copy);

  host.append(spinner);
  document.body.append(host);

  for (const [element, scrollTop] of frozen?.scrolls ?? []) element.scrollTop = scrollTop;
}

function hideReconnecting() {
  document.getElementById(HOST_ID)?.remove();
}
