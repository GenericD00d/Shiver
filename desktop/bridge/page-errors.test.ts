import { expect, test } from 'bun:test';

import { describe, note, redact, takePageErrors } from './page-errors';

test('an error reads as its name, message and first stack line', () => {
  const error = new TypeError('desktopApi.setVoiceActive is not a function');

  error.stack = 'TypeError: desktopApi.setVoiceActive is not a function\n    at DesktopController (index-D89i86R5.js:1:2)\n    at x';

  expect(describe(error)).toBe(
    'TypeError: desktopApi.setVoiceActive is not a function (at DesktopController (index-D89i86R5.js:1:2))'
  );
  expect(describe({ code: 4 })).toBe('{"code":4}');
});

test('tokens are blanked and a line stays one line', () => {
  expect(redact('GET https://chat.example/public/a.png?accessToken=eyJabc.def&w=2 failed')).toBe(
    'GET https://chat.example/public/a.png?accessToken=…&w=2 failed'
  );
  expect(redact('x?token=abc y&JWT=def')).toBe('x?token=… y&JWT=…');
  expect(redact('Authorization: Bearer eyJ.abc')).toBe('Authorization: Bearer …');
  expect(redact('one\n  two')).toBe('one ⏎ two');
  expect(redact('a'.repeat(5000))).toHaveLength(1000);
});

test('errors wait for the drain, and a flood is counted rather than kept', () => {
  takePageErrors();
  note('first');

  expect(takePageErrors()).toEqual(['first']);
  expect(takePageErrors()).toEqual([]);

  for (let i = 0; i < 40; i++) note(`error ${i}`);

  const taken = takePageErrors();

  expect(taken).toHaveLength(31);
  expect(taken.at(-1)).toBe('…and 10 more');
});
