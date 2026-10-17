/**
 * Sharkord's own channel mute (0.0.27 and newer) as the face of Shiver's. Sharkord keeps its mutes
 * in this browser's storage only, read once when its client loads and written on every toggle from
 * its channel menu or top bar; Shiver's list is the one that follows the user to their other devices
 * (through the companion plugin) and reaches the core's notifications and badges.
 *
 * So Shiver writes its list into Sharkord's key, and Sharkord's next load shows it; and each write
 * Sharkord makes is compared with what Sharkord last read or wrote, which names exactly the channel
 * the user toggled. A change that reaches Shiver from elsewhere while the page is open is in
 * Shiver's list at once, and in Sharkord's menu and dimming from its next load.
 */

export const SHARKORD_MUTES_KEY = 'sharkord-muted-channel-ids';

const INSTALLED = Symbol.for('shiver.sharkordMutes');

type Toggled = (channelId: number, muted: boolean) => void;

type Installed = {
  /** what Sharkord holds: the list it last read or wrote */
  view: Set<number> | null;
  /** Sharkord has read its list (it does so once, as it loads); any later read is someone else's */
  read: boolean;
  /** Storage's own `setItem`, which Shiver's writes use so that they are not taken for Sharkord's */
  write: Storage['setItem'];
  listeners: Toggled[];
};

/** A stored list as Sharkord reads it: an array of channel ids; anything else is none. */
export const readMutes = (stored: string | null): Set<number> => {
  try {
    const parsed: unknown = stored === null ? [] : JSON.parse(stored);

    return new Set(Array.isArray(parsed) ? parsed.filter((id): id is number => Number.isSafeInteger(id) && id > 0) : []);
  } catch {
    return new Set();
  }
};

/** The channels whose state differs between `before` and `after`, each with its state in `after`. */
export const toggledBetween = (before: ReadonlySet<number>, after: ReadonlySet<number>): [number, boolean][] => [
  ...[...after].filter((id) => !before.has(id)).map((id): [number, boolean] => [id, true]),
  ...[...before].filter((id) => !after.has(id)).map((id): [number, boolean] => [id, false])
];

const installed = () => (window as unknown as { [INSTALLED]?: Installed })[INSTALLED];

/**
 * Watches Sharkord read and write its mutes, telling `onToggled` of each channel the user mutes or
 * unmutes in Sharkord. Must run before Sharkord's scripts to see its first read; installed later,
 * the stored list stands in for it, which is what Sharkord read unless Shiver wrote since. A script
 * that runs first can install it with no listener, and the bridge add its listener later.
 */
export function watchSharkordMutes(onToggled?: Toggled) {
  const existing = installed();

  if (existing) {
    if (onToggled) existing.listeners.push(onToggled);

    return;
  }

  const prototype = Storage.prototype;
  const getItem = prototype.getItem;
  const setItem = prototype.setItem;
  const state: Installed = { view: null, read: false, write: setItem, listeners: onToggled ? [onToggled] : [] };

  (window as unknown as { [INSTALLED]?: Installed })[INSTALLED] = state;

  const isSharkords = (storage: Storage, key: string) => key === SHARKORD_MUTES_KEY && storage === window.localStorage;

  prototype.getItem = function (this: Storage, key: string) {
    const value = getItem.call(this, key);

    if (!state.read && isSharkords(this, key)) {
      state.read = true;
      state.view = readMutes(value);
    }

    return value;
  };

  prototype.setItem = function (this: Storage, key: string, value: string) {
    setItem.call(this, key, value);

    if (!isSharkords(this, key)) return;

    const before = state.view ?? readMutes(null);
    const after = readMutes(String(value));

    state.view = after;

    for (const [channelId, muted] of toggledBetween(before, after)) {
      for (const listener of state.listeners) listener(channelId, muted);
    }
  };

  try {
    state.view = readMutes(getItem.call(window.localStorage, SHARKORD_MUTES_KEY));
    // installed after Sharkord loaded (its store is up), so its one read is already behind it
    state.read = !!window.__SHARKORD_STORE__;
  } catch {
    // storage refused: Sharkord cannot keep mutes either
  }
}

/** Writes Shiver's list into Sharkord's key, for its next load. Not one of Sharkord's own writes. */
export function seedSharkordMutes(muted: Iterable<number>) {
  const write = installed()?.write ?? Storage.prototype.setItem;

  try {
    write.call(window.localStorage, SHARKORD_MUTES_KEY, JSON.stringify([...muted].sort((a, b) => a - b)));
  } catch {
    // storage refused: Sharkord's menu then starts from nothing muted
  }
}
