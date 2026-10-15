/**
 * The rail page's one way out: `shiverRail`, the channel tauri-plugin-shiver-rail offers this page's
 * origin. Everything the plugin and the core send arrives here, and is handed on by its shape:
 *
 * - `{ seq, reveal, view }`: what to draw (`reveal`: the rail is sliding in, so start on the rail)
 * - `{ reply, ok, value }`: the answer to one of this page's calls
 * - `{ event, payload }`: one of the core's events
 * - `{ back: true }`: back was pressed while this page fills the overlay
 *
 * The page has no IPC, so this module stands in for `@tauri-apps/api`'s `invoke` and `listen` in
 * its build (`vite.rail.config.ts`): Shiver's screens and their `api` calls run here unchanged, and
 * the core takes only the commands it lists for this page (`overlay.rs`).
 */

/** What the page says. The plugin passes on only these kinds, and the core checks each. */
export type Said =
  | { kind: 'ready' }
  | { kind: 'drawn'; seq: number }
  | { kind: 'open'; entryId: string; dmUser?: string }
  | { kind: 'add' | 'closed' }
  | { kind: 'folder'; folderId: string; expanded: boolean }
  /** fill the overlay (a menu or a screen), or go back to the rail's width */
  | { kind: 'expand'; full: boolean }
  | { kind: 'call'; id: number; cmd: string; args: Record<string, unknown> };

type Channel = {
  postMessage: (message: string) => void;
  onmessage: ((event: { data: string }) => void) | null;
};

declare global {
  interface Window {
    shiverRail?: Channel;
  }
}

/** How long a call may go unanswered before it fails (a sign-in takes up to 20 s). */
const CALL_TIMEOUT_MS = 60_000;

export const say = (message: Said) => window.shiverRail?.postMessage(JSON.stringify(message));

type Handler = (message: Record<string, unknown>) => void;

const viewHandlers = new Set<Handler>();
const backHandlers = new Set<() => void>();
const eventHandlers = new Map<string, Set<(event: { payload: unknown }) => void>>();
const pending = new Map<number, { resolve: (value: unknown) => void; reject: (cause: unknown) => void }>();
let nextCall = 0;

/** Hears each view the plugin sends, `{ seq, reveal, view }`. */
export const onView = (handler: Handler) => {
  viewHandlers.add(handler);

  return () => void viewHandlers.delete(handler);
};

/** Hears back presses while the page fills the overlay. */
export const onBack = (handler: () => void) => {
  backHandlers.add(handler);

  return () => void backHandlers.delete(handler);
};

const heard = (data: string) => {
  let message: Record<string, unknown>;

  try {
    message = JSON.parse(data);
  } catch {
    return;
  }

  if (!message || typeof message !== 'object') return;

  if (typeof message.reply === 'number') {
    const call = pending.get(message.reply);

    if (!call) return;

    pending.delete(message.reply);

    if (message.ok === true) call.resolve(message.value);
    else call.reject(message.value);

    return;
  }

  if (typeof message.event === 'string') {
    for (const handler of eventHandlers.get(message.event) ?? []) handler({ payload: message.payload });

    return;
  }

  if (message.back === true) {
    for (const handler of backHandlers) handler();

    return;
  }

  if (typeof message.seq === 'number') {
    for (const handler of viewHandlers) handler(message);
  }
};

if (window.shiverRail) window.shiverRail.onmessage = ({ data }) => heard(data);

/** `@tauri-apps/api/core`'s `invoke`, over the channel. Rejects with the core's message, as Tauri does. */
export function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!window.shiverRail) return Promise.reject('Shiver is not reachable from here');

  const id = nextCall++;

  return new Promise<T>((resolve, reject) => {
    const timer = window.setTimeout(() => {
      if (pending.delete(id)) reject('Shiver did not answer');
    }, CALL_TIMEOUT_MS);

    pending.set(id, {
      resolve: (value) => {
        window.clearTimeout(timer);
        resolve(value as T);
      },
      reject: (cause) => {
        window.clearTimeout(timer);
        reject(cause);
      }
    });

    say({ kind: 'call', id, cmd, args });
  });
}

/** `@tauri-apps/api/event`'s `listen`, for the core's events while this page is up. */
export function listen<T>(event: string, handler: (event: { payload: T }) => void): Promise<() => void> {
  const handlers = eventHandlers.get(event) ?? new Set();
  const typed = handler as (event: { payload: unknown }) => void;

  handlers.add(typed);
  eventHandlers.set(event, handlers);

  return Promise.resolve(() => void handlers.delete(typed));
}
