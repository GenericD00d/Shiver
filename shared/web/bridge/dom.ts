/** DOM helpers shared by the desktop and mobile bridges. */

/** The `<style>` element with `id`, created on first use. */
export function ensureStyle(id: string): HTMLStyleElement {
  const existing = document.getElementById(id);

  if (existing instanceof HTMLStyleElement) return existing;

  const style = document.createElement('style');

  style.id = id;
  document.documentElement.appendChild(style);

  return style;
}

/**
 * Defines a `window` hook the core reads or calls, non-writable and non-configurable so the page
 * cannot replace it. A name the page locked first (possible with mobile's post-load injection)
 * keeps the page's value; the core treats everything it reads back as a request anyway.
 */
export function defineHook<K extends keyof Window>(name: K, value: Window[K]) {
  try {
    delete window[name];
    Object.defineProperty(window, name, { value, writable: false, configurable: false, enumerable: false });
  } catch {
    // already defined by an earlier install, or locked by the page
  }
}

const settledCallbacks = new Set<() => void>();
let settledObserver: MutationObserver | null = null;
let settledScheduled = false;

function runSettled() {
  settledScheduled = false;

  for (const callback of settledCallbacks) {
    try {
      callback();
    } catch (error) {
      console.error('[shiver] a dom-settled callback failed', error);
    }
  }
}

/**
 * Runs `callback` after DOM changes, at most once per animation frame. One shared observer for
 * the whole bridge, so a burst of mutations costs one pass; callbacks must be idempotent.
 */
export function onDomSettled(callback: () => void) {
  settledCallbacks.add(callback);

  if (settledObserver) return;

  settledObserver = new MutationObserver(() => {
    if (settledScheduled) return;

    settledScheduled = true;
    requestAnimationFrame(runSettled);
  });

  settledObserver.observe(document.body, { childList: true, subtree: true });
}

/** Whether this is the top frame (a cross-origin parent makes `window.top` throw). */
export function isTopFrame() {
  try {
    return window.top === window.self;
  } catch {
    return false;
  }
}

export function whenDocumentReady(run: () => void) {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', run, { once: true });
  } else {
    run();
  }
}

/** Sharkord's open (Radix) menu, if one is on screen; closed menus can linger in the document. */
export function openMenuOnScreen(): HTMLElement | null {
  for (const menu of document.querySelectorAll<HTMLElement>('[role="menu"]')) {
    if (menu.dataset.state === 'closed') continue;

    const box = menu.getBoundingClientRect();

    if (box.width > 0 && box.height > 0) return menu;
  }

  return null;
}

/** The `[role="menu"]` a mutation added, if any. */
export function addedMenu(records: MutationRecord[]): HTMLElement | null {
  for (const record of records) {
    for (const node of record.addedNodes) {
      if (!(node instanceof HTMLElement)) continue;

      const menu = node.matches('[role="menu"]') ? node : node.querySelector<HTMLElement>('[role="menu"]');

      if (menu) return menu;
    }
  }

  return null;
}

const CONTROL = 'button, input, select, textarea, label, [role="button"], [contenteditable]';

/**
 * Catches plain clicks on `target="_blank"` http(s) links, which the webview will not open as a
 * new window, and hands the address to `open` (the core opens it in the browser). Controls inside
 * a link (the delete button on an attachment card) are left to the page.
 */
export function installExternalLinks(open: (href: string) => void) {
  document.addEventListener(
    'click',
    (event) => {
      if (!event.isTrusted || event.defaultPrevented) return;
      if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;

      const target = event.target as Element | null;
      const anchor = target?.closest?.('a');

      if (!(anchor instanceof HTMLAnchorElement) || anchor.target !== '_blank') return;

      const control = target?.closest?.(CONTROL);

      if (control && control !== anchor && anchor.contains(control)) return;
      if (!/^https?:/i.test(anchor.href)) return;

      event.preventDefault();
      event.stopPropagation();
      open(anchor.href);
    },
    true
  );
}
