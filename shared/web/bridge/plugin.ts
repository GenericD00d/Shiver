/**
 * Talking to Shiver's companion plugin from a server's page. The bridge cannot call
 * `executePluginAction` itself (Sharkord reads the plugin id off the calling stack), so requests
 * are posted into the page and the plugin's client bundle relays them to its server actions.
 */

import { SHIVER_PLUGIN_ID, sharkordStore } from './sharkord';

declare global {
  interface Window {
    /** set by the plugin's client bundle; version 2 relays every write as a server action */
    __SHIVER_PLUGIN__?: { version: number };
  }
}

type PluginResult = { mutedChannels?: number[]; status?: string } | null;

type PluginResponse = { source?: string; id?: string; ok?: boolean; result?: PluginResult };

const REQUEST = 'shiver-bridge';
const RESPONSE = 'shiver-plugin';
const TIMEOUT_MS = 8000;
/** the client imports plugin bundles after connecting, so the relay appears late */
const WAIT_MS = 15000;
/** this device's own mutes have been merged into the plugin's list once */
const MUTES_ADOPTED = 'shiver-mutes-adopted';

/** Calls a relay action; null when there is no plugin, it fails, or it times out. */
export function callPlugin(action: string, payload?: unknown): Promise<PluginResult> {
  if (!window.__SHIVER_PLUGIN__) return Promise.resolve(null);

  return new Promise((resolve) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;

    const done = (value: PluginResult) => {
      window.removeEventListener('message', onMessage);
      window.clearTimeout(timer);
      resolve(value);
    };

    const onMessage = (event: MessageEvent) => {
      if (event.source !== window || event.origin !== window.location.origin) return;

      const data = event.data as PluginResponse | undefined;

      if (data?.source === RESPONSE && data.id === id) done(data.ok ? (data.result ?? null) : null);
    };

    const timer = window.setTimeout(() => done(null), TIMEOUT_MS);

    window.addEventListener('message', onMessage);
    window.postMessage({ source: REQUEST, id, action, payload }, window.location.origin);
  });
}

let waiting: Promise<boolean> | null = null;

/**
 * Resolves true once the relay is present, false after `WAIT_MS` without it. Callers waiting at the
 * same time share one wait.
 */
export function waitForPlugin(): Promise<boolean> {
  if (window.__SHIVER_PLUGIN__) return Promise.resolve(true);

  waiting ??= new Promise<boolean>((resolve) => {
    const started = Date.now();
    const timer = window.setInterval(() => {
      const present = !!window.__SHIVER_PLUGIN__;

      if (present || Date.now() - started > WAIT_MS) {
        window.clearInterval(timer);
        resolve(present);
      }
    }, 500);
  }).finally(() => {
    waiting = null;
  });

  return waiting;
}

const adoptedMutes = () => {
  try {
    return localStorage.getItem(MUTES_ADOPTED) === '1';
  } catch {
    // unknown: re-adopting is the worse failure
    return true;
  }
};

const rememberAdoptedMutes = () => {
  try {
    localStorage.setItem(MUTES_ADOPTED, '1');
  } catch {
    // at worst one more merge next time
  }
};

/**
 * Reconciles mutes with the plugin's copy once connected. The plugin's list is the truth (a union
 * on every connection would make unmutes impossible to propagate), except the first time this
 * device meets the plugin, when its local mutes are merged in and written back once. Returns the
 * list to use, or null without a plugin.
 */
export async function syncMutesWithPlugin(local: number[]): Promise<number[] | null> {
  if (!(await waitForPlugin())) return null;

  const remote = await callPlugin('getMutedChannels');

  if (!remote) return null;

  const theirs = (remote.mutedChannels ?? []).filter((id): id is number => Number.isInteger(id) && id > 0);

  if (adoptedMutes()) return [...theirs].sort((a, b) => a - b);

  const merged = [...new Set([...theirs, ...local])].sort((a, b) => a - b);

  if (merged.length !== theirs.length) await callPlugin('setMutedChannels', { mutedChannels: merged });

  rememberAdoptedMutes();

  return merged;
}

export function pushMutesToPlugin(mutedChannels: number[]) {
  void callPlugin('setMutedChannels', { mutedChannels });
}

/**
 * Stores this user's unread floor for this server in the plugin, so their devices share one badge
 * (the core reads it back when it connects). Silent without a plugin.
 */
export async function storeReadFloor(floor: Record<string, number>) {
  if (!(await waitForPlugin())) return;

  if ((window.__SHIVER_PLUGIN__?.version ?? 1) >= 2) {
    await callPlugin('setReadFloor', { floor });

    return;
  }

  // version 1 has no action for it: read-modify-write the row, keeping the mute list in it
  const actions = sharkordStore()?.actions;

  if (!actions?.getUserData || !actions.setUserData) return;

  try {
    const stored = (await actions.getUserData(SHIVER_PLUGIN_ID)) ?? {};

    await actions.setUserData(SHIVER_PLUGIN_ID, { ...stored, readFloor: floor });
  } catch {
    // plugin switched off
  }
}
