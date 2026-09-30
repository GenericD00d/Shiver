import { expect, test } from 'bun:test';

import { dmInitial, dmKey, dmMeta, filterDms } from './dms';
import { relativeTime } from './time';

const rows = [
  { name: 'Ana', serverName: 'Chat' },
  { name: 'Bob', serverName: 'Work' },
  { name: 'Cleo', serverName: 'Anarchy' }
];

test('a search matches the person or the server, ignoring case and spaces around it', () => {
  expect(filterDms(rows, '').map((row) => row.name)).toEqual(['Ana', 'Bob', 'Cleo']);
  expect(filterDms(rows, ' ana ').map((row) => row.name)).toEqual(['Ana', 'Cleo']);
  expect(filterDms(rows, 'WORK').map((row) => row.name)).toEqual(['Bob']);
  expect(filterDms(rows, 'nobody')).toEqual([]);
});

test('a row names its account only when there is one', () => {
  expect(dmMeta({ accountLabel: 'alt', serverName: 'Chat' })).toBe('alt · Chat');
  expect(dmMeta({ accountLabel: null, serverName: 'Chat' })).toBe('Chat');
  expect(dmMeta({ accountLabel: '', serverName: 'Chat' })).toBe('Chat');
});

test('an avatar without a picture shows the first letter, whole', () => {
  expect(dmInitial(' ana')).toBe('A');
  expect(dmInitial('😀 smile')).toBe('😀');
  expect(dmInitial('  ')).toBe('?');
  expect(dmKey('entry', 7)).toBe('entry:7');
});

test('times read as how long ago', () => {
  const now = 1_000_000_000;

  expect(relativeTime(now - 5_000, now)).toBe('just now');
  expect(relativeTime(now + 5_000, now)).toBe('just now');
  expect(relativeTime(now - 5 * 60_000, now)).toBe('5m ago');
  expect(relativeTime(now - 3 * 3_600_000, now)).toBe('3h ago');
  expect(relativeTime(now - 2 * 86_400_000, now)).toBe('2d ago');
});
