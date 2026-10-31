import { expect, test } from 'bun:test';

import { contrastRatio, readableAccent } from './colors';

test('contrast runs from 1 to 21', () => {
  expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21);
  expect(contrastRatio('#123456', '#123456')).toBe(1);
  expect(contrastRatio('red', '#ffffff')).toBe(1);
});

test('an accent that reads on the background is kept', () => {
  expect(readableAccent('#e5e5e5', '#0a0a0a', '#fafafa')).toBe('#e5e5e5');
  expect(readableAccent('#5865f2', '#ffffff', '#171717')).toBe('#5865f2');
});

test('an accent too close to the background moves towards the text until it reads', () => {
  const accent = readableAccent('#18181a', '#1f1f25', '#fafafa');

  expect(accent).not.toBe('#18181a');
  expect(contrastRatio(accent, '#1f1f25')).toBeGreaterThanOrEqual(4.5);
  expect(readableAccent('#18181a', '#1f1f25', '#1f1f25')).toBe('#1f1f25');
});
