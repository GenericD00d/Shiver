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

const settledCallbacks = new Set<(changed: readonly Element[]) => void>();
let settledObserver: MutationObserver | null = null;
let settledScheduled = false;
/** elements added or re-texted since the last run; the body alone once there are too many to track */
const settledChanged = new Set<Element>();
const MAX_TRACKED = 500;

function runSettled() {
  settledScheduled = false;

  const changed = [...settledChanged].filter((element) => element.isConnected && !hasAncestorIn(element, settledChanged));

  settledChanged.clear();

  for (const callback of settledCallbacks) {
    try {
      callback(changed);
    } catch (error) {
      console.error('[shiver] a dom-settled callback failed', error);
    }
  }
}

function hasAncestorIn(element: Element, set: ReadonlySet<Element>) {
  for (let up = element.parentElement; up; up = up.parentElement) if (set.has(up)) return true;

  return false;
}

function noteChanged(node: Node | null) {
  const element = node instanceof Element ? node : node?.parentElement;

  if (!element || settledChanged.has(document.body)) return;

  if (settledChanged.size < MAX_TRACKED) {
    settledChanged.add(element);

    return;
  }

  settledChanged.clear();
  settledChanged.add(document.body);
}

/**
 * Runs `callback` after DOM changes, at most once per animation frame, with the outermost elements
 * added or re-texted since the last run, so it can look at just those (`touched`). One shared
 * observer for the whole bridge; callbacks must be idempotent.
 */
export function onDomSettled(callback: (changed: readonly Element[]) => void) {
  settledCallbacks.add(callback);

  if (settledObserver) return;

  settledObserver = new MutationObserver((records) => {
    for (const record of records) {
      if (record.type === 'characterData') noteChanged(record.target);
      else for (const node of record.addedNodes) noteChanged(node);
    }

    if (settledScheduled) return;

    settledScheduled = true;
    requestAnimationFrame(runSettled);
  });

  settledObserver.observe(document.body, { childList: true, subtree: true, characterData: true });
}

/** Elements matching `selector` among, inside or enclosing (the nearest) the `changed` ones. */
export function touched<T extends Element = HTMLElement>(changed: readonly Element[], selector: string) {
  const found = new Set<T>();

  for (const root of changed) {
    const around = root.closest<T>(selector);

    if (around) found.add(around);

    for (const inner of root.querySelectorAll<T>(selector)) found.add(inner);
  }

  return found;
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
 * Hands `open` (which the core rations and opens in the browser) what the user opens away from the
 * page: a trusted click on an http(s) link to another origin, into a new window or with a modifier,
 * and one `window.open` per gesture. The core opens nothing else. Controls inside a link (the
 * delete button on an attachment card) are left to the page.
 */
export function installExternalLinks(open: (href: string) => void) {
  let armed = false;

  const capture = (event: MouseEvent) => {
    if (!event.isTrusted || event.defaultPrevented || event.button > 1) return;

    const target = event.target as Element | null;
    const anchor = target?.closest?.('a');

    if (!(anchor instanceof HTMLAnchorElement) || !/^https?:$/.test(anchor.protocol)) return;

    const control = target?.closest?.(CONTROL);

    if (control && control !== anchor && anchor.contains(control)) return;
    if (anchor.target !== '_blank' && anchor.origin === location.origin && event.button === 0 && !(event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)) return;

    event.preventDefault();
    event.stopPropagation();
    open(anchor.href);
  };

  document.addEventListener('click', capture, true);
  document.addEventListener('auxclick', capture, true);

  for (const type of ['pointerdown', 'keydown']) window.addEventListener(type, (event) => (armed ||= event.isTrusted), true);

  window.open = (url?: string | URL) => {
    const href = armed && navigator.userActivation?.isActive && url ? new URL(url, location.href) : null;

    armed = false;

    if (href && /^https?:$/.test(href.protocol)) open(href.href);

    return null;
  };
}
