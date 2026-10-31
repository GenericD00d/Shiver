import { expect, test } from 'bun:test';

import { contrastRatio, liftHex, readableAccent } from './colors';

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

test('a lifted colour as hex moves dark colours towards white and light ones towards black', () => {
  expect(liftHex('#000000', 90)).toBe('#1a1a1a');
  expect(liftHex('#ffffff', 90)).toBe('#e6e6e6');
  expect(liftHex('#123456', 100)).toBe('#123456');
  expect(liftHex('red', 90)).toBe('red');
});
