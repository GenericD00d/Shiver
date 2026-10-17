import { expect, test } from 'bun:test';

import { readMutes, seedSharkordMutes, SHARKORD_MUTES_KEY, toggledBetween, watchSharkordMutes } from './mutes';

/** Just enough of the Web Storage API, on its own prototype as the browser's is. */
class FakeStorage {
  items = new Map<string, string>();

  getItem(key: string) {
    return this.items.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    this.items.set(key, String(value));
  }
}

test('a stored list reads as channel ids, and anything else as none', () => {
  expect([...readMutes('[3, 1, "x", -2, 1.5, 3]')]).toEqual([3, 1]);
  expect(readMutes('{"not": "a list"}').size).toBe(0);
  expect(readMutes('not json').size).toBe(0);
  expect(readMutes(null).size).toBe(0);
});

test('two lists differ by the channels toggled, each with its new state', () => {
  expect(toggledBetween(new Set([1, 2]), new Set([2, 3]))).toEqual([
    [3, true],
    [1, false]
  ]);
  expect(toggledBetween(new Set([1]), new Set([1]))).toEqual([]);
});

test("Sharkord's toggles are told apart from Shiver's own writes, against what Sharkord holds", () => {
  const storage = new FakeStorage();

  (globalThis as { Storage?: unknown }).Storage = FakeStorage;
  (globalThis as { window?: unknown }).window = { localStorage: storage };

  const toggled: [number, boolean][] = [];

  // installed before Sharkord loads, which then reads what Shiver seeded
  watchSharkordMutes((channelId, muted) => toggled.push([channelId, muted]));
  seedSharkordMutes([5, 4]);
  // Sharkord loading
  expect(storage.getItem(SHARKORD_MUTES_KEY)).toBe('[4,5]');

  // Sharkord's own write: channel 6 muted on top of what it read
  storage.setItem(SHARKORD_MUTES_KEY, '[4,5,6]');
  expect(toggled).toEqual([[6, true]]);

  // Shiver learns of 9 from another device: written for Sharkord's next load, not taken as a toggle
  seedSharkordMutes([4, 5, 6, 9]);
  expect(toggled.length).toBe(1);

  // Sharkord still holds [4,5,6], so its next toggle (5 unmuted) does not unmute 9
  storage.setItem(SHARKORD_MUTES_KEY, '[4,6]');
  expect(toggled).toEqual([
    [6, true],
    [5, false]
  ]);

  // a later read (devtools, a plugin) is not Sharkord's: it still holds [4,6]
  seedSharkordMutes([4, 6, 9]);
  storage.getItem(SHARKORD_MUTES_KEY);
  storage.setItem(SHARKORD_MUTES_KEY, '[6]');
  expect(toggled).toEqual([
    [6, true],
    [5, false],
    [4, false]
  ]);

  // other keys and other storages are not Sharkord's mutes
  storage.setItem('another-key', '[1]');
  new FakeStorage().setItem(SHARKORD_MUTES_KEY, '[1]');
  expect(toggled.length).toBe(3);
});
