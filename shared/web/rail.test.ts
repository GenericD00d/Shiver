import { describe, expect, test } from 'bun:test';

import { byPosition, initials, membersOf, railOrder } from './rail';

describe('rail helpers', () => {
  test('initials take two words and fall back to ?', () => {
    expect(initials('my  cool server')).toBe('MC');
    expect(initials('   ')).toBe('?');
  });

  test('ordering copies rather than sorting in place', () => {
    const items = [{ position: 2 }, { position: 0 }];

    expect(byPosition(items).map((item) => item.position)).toEqual([0, 2]);
    expect(items[0].position).toBe(2);
  });

  test('folder members come in their own order', () => {
    const servers = [
      { id: 'a', position: 1, folderId: 'f' },
      { id: 'b', position: 0, folderId: 'f' },
      { id: 'c', position: 0, folderId: null }
    ];

    expect(membersOf(servers, 'f').map((server) => server.id)).toEqual(['b', 'a']);
  });

  test('the rail order puts a folder’s servers where the folder sits', () => {
    const servers = [
      { id: 'a', position: 1, folderId: 'f' },
      { id: 'b', position: 0, folderId: 'f' },
      { id: 'c', position: 0, folderId: null },
      { id: 'd', position: 2, folderId: null },
      { id: 'e', position: 0, folderId: 'gone' }
    ];

    expect(railOrder(servers, [{ id: 'f', position: 1 }]).map((server) => server.id)).toEqual(['c', 'b', 'a', 'd']);
  });
});
