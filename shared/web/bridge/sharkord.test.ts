import { expect, test } from 'bun:test';

import { dmPartnerId, fileUrl, findDmChannelIdByUserName, notificationTarget, readDms, rowName, type SharkordState } from './sharkord';

const ORIGIN = 'https://chat.example';

/** Signed in as user 1, with DMs with Ana (2) and Ana B (3), and a channel whose name has ` in #`. */
const state: SharkordState = {
  ownUserId: 1,
  users: [
    { id: 1, name: 'Me' },
    { id: 2, name: 'Ana', avatar: { name: 'avatars/a b.png', _accessToken: 't0k', _accessTokenExpiresAt: 99 } },
    { id: 3, name: 'Ana B' },
    { id: 4, name: 'x in #general' }
  ],
  channels: [
    { id: 10, name: 'general', type: 'TEXT' },
    { id: 11, name: 'news in #general', type: 'TEXT' },
    { id: 20, name: 'DM - 1:2', isDm: true },
    { id: 21, name: 'DM - 3:1', isDm: true },
    { id: 22, name: 'DM - 1:9', isDm: true }
  ]
};

test('a DM partner is whichever side of the channel name is not the user', () => {
  expect(dmPartnerId({ id: 20, name: 'DM - 1:2' }, 1)).toBe(2);
  expect(dmPartnerId({ id: 21, name: 'DM - 3:1' }, 1)).toBe(3);
  expect(dmPartnerId({ id: 21, name: 'DM - 3:1' }, undefined)).toBe(3);
  expect(dmPartnerId({ id: 10, name: 'general' }, 1)).toBeNull();
  expect(dmPartnerId({ id: 10, name: 'DM - 1:2 ' }, 1)).toBeNull();
});

test('a DM is found by its partner’s exact name', () => {
  expect(findDmChannelIdByUserName(state, 'Ana')).toBe(20);
  expect(findDmChannelIdByUserName(state, 'Ana B')).toBe(21);
  expect(findDmChannelIdByUserName(state, 'ana')).toBeNull();
  expect(findDmChannelIdByUserName({}, 'Ana')).toBeNull();
});

test('notification titles name the author and the channel or DM', () => {
  expect(notificationTarget('Ana (DM)', state)).toEqual({ channelId: 20, channelName: null, author: 'Ana', isDm: true });
  expect(notificationTarget('Ana in #general', state)).toEqual({ channelId: 10, channelName: 'general', author: 'Ana', isDm: false });
  // a DM from a user not in the store yet: resolved later, from the author
  expect(notificationTarget('Zed (DM)', state)).toEqual({ channelId: null, channelName: null, author: 'Zed', isDm: true });
  expect(notificationTarget('Ana in #gone', state)).toEqual({ channelId: null, channelName: 'gone', author: 'Ana', isDm: false });
  expect(notificationTarget('something else', state)).toEqual({ channelId: null, channelName: null, author: 'something else', isDm: false });
});

test('names containing the title’s own separators still resolve', () => {
  expect(notificationTarget('x in #general in #general', state)).toMatchObject({ channelId: 10, author: 'x in #general' });
  expect(notificationTarget('Ana in #news in #general', state)).toMatchObject({ channelId: 11, author: 'Ana' });
  // only a trailing marker makes a DM
  expect(notificationTarget('Bob (DM) in #general', state)).toMatchObject({ channelId: 10, author: 'Bob (DM)', isDm: false });
});

test('file urls are encoded per segment and carry their access token', () => {
  expect(fileUrl(ORIGIN, { name: 'avatars/a b.png', _accessToken: 't0k', _accessTokenExpiresAt: 99 })).toBe(
    'https://chat.example/public/avatars/a%20b.png?accessToken=t0k&expires=99'
  );
  expect(fileUrl(ORIGIN, { name: 'x?y#z' })).toBe('https://chat.example/public/x%3Fy%23z');

  for (const name of ['../x', 'a/../../x', './x', 'a//b', '/x', '']) expect(fileUrl(ORIGIN, { name })).toBeNull();

  expect(fileUrl(ORIGIN, null)).toBeNull();
  expect(fileUrl('not an origin', { name: 'a.png' })).toBeNull();
});

test('DMs list newest first, then by name, skipping partners not loaded yet', () => {
  expect(readDms(ORIGIN, state, new Map()).map(({ name }) => name)).toEqual(['Ana', 'Ana B']);

  const listed = readDms(ORIGIN, state, new Map([[21, 5]]));

  expect(listed.map(({ channelId, lastMessageAt }) => [channelId, lastMessageAt])).toEqual([
    [21, 5],
    [20, null]
  ]);
  expect(listed[1].iconUrl).toBe('https://chat.example/public/avatars/a%20b.png?accessToken=t0k&expires=99');
});

/** A sidebar row: its text, and optionally its name span and unread pill. */
const row = (text: string, { name, count }: { name?: string; count?: string } = {}) =>
  ({
    textContent: text,
    querySelector: (selector: string) => {
      if (selector.startsWith('span.')) return name === undefined ? null : { textContent: name };
      if (selector.includes('unread-count')) return count === undefined ? null : { textContent: count };

      return null;
    }
  }) as unknown as Element;

test('a row’s name is its name span, else its text without the unread pill', () => {
  expect(rowName(row('general3', { name: ' general ', count: '3' }))).toBe('general');
  expect(rowName(row(' general3 ', { count: '3' }))).toBe('general');
  expect(rowName(row('room 42', { count: '7' }))).toBe('room 42');
  expect(rowName(row(''))).toBe('');
});
