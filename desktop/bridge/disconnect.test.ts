import { expect, test } from 'bun:test';

import { createDisconnectWatch, DISCONNECT_WINDOW_MS } from './disconnect';

const watchAt = () => {
  let now = 1_000_000;
  const watch = createDisconnectWatch(() => now);

  return { watch, wait: (ms: number) => (now += ms) };
};

test('a connect form with no Disconnect chosen is not the user disconnecting', () => {
  const { watch } = watchAt();

  expect(watch.take(true)).toBe(false);
});

test('the connect form after choosing Disconnect is, once', () => {
  const { watch, wait } = watchAt();

  watch.chosen();
  expect(watch.take(false)).toBe(false);

  wait(5_000);
  expect(watch.take(true)).toBe(true);
  expect(watch.take(true)).toBe(false);
});

test('a choice left too long was cancelled', () => {
  const { watch, wait } = watchAt();

  watch.chosen();
  wait(DISCONNECT_WINDOW_MS);
  expect(watch.take(false)).toBe(false);

  // the stale choice is gone, so a refusal later is not read as the user's
  expect(watch.take(true)).toBe(false);
});

test('a clock that went back does not stretch the window', () => {
  const { watch, wait } = watchAt();

  watch.chosen();
  wait(-1);
  expect(watch.take(true)).toBe(false);
});
