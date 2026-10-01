import { describe, expect, test } from 'bun:test';

import { byPosition, initials, membersOf, railDrop, railOrder } from './rail';

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

describe('railDrop', () => {
  // top level: a(0), folder f(1) holding b, c, d(2); folder g(3) holding e, h
  const servers = [
    { id: 'a', position: 0, folderId: null },
    { id: 'b', position: 0, folderId: 'f' },
    { id: 'c', position: 1, folderId: 'f' },
    { id: 'd', position: 2, folderId: null },
    { id: 'e', position: 0, folderId: 'g' },
    { id: 'h', position: 1, folderId: 'g' }
  ];
  const folders = [
    { id: 'f', position: 1 },
    { id: 'g', position: 3 }
  ];
  const server = (id: string) => ({ kind: 'server' as const, id });
  const folder = (id: string) => ({ kind: 'folder' as const, id });

  test('onto a loose server makes a folder of the two, target first', () => {
    expect(railDrop(servers, folders, server('a'), server('d'), 'into')).toEqual([
      { op: 'createFolder', memberIds: ['d', 'a'] }
    ]);
  });

  test('into a folder moves a server in; folders and servers in folders take nothing', () => {
    expect(railDrop(servers, folders, server('a'), folder('f'), 'into')).toEqual([
      { op: 'setFolder', serverId: 'a', folderId: 'f' }
    ]);
    expect(railDrop(servers, folders, server('b'), folder('f'), 'into')).toEqual([]);
    expect(railDrop(servers, folders, folder('f'), server('d'), 'into')).toEqual([]);
    expect(railDrop(servers, folders, server('a'), server('b'), 'into')).toEqual([]);
  });

  test('the top level reorders folders and servers together', () => {
    expect(railDrop(servers, folders, server('d'), server('a'), 'before')).toEqual([
      { op: 'reorder', ordered: [server('d'), server('a'), folder('f'), folder('g')] }
    ]);
    expect(railDrop(servers, folders, folder('f'), folder('g'), 'after')).toEqual([
      { op: 'reorder', ordered: [server('a'), server('d'), folder('g'), folder('f')] }
    ]);
  });

  test('beside a server in a folder joins that folder, in place', () => {
    expect(railDrop(servers, folders, server('a'), server('c'), 'before')).toEqual([
      { op: 'setFolder', serverId: 'a', folderId: 'f' },
      { op: 'reorderInFolder', ids: ['b', 'a', 'c'] }
    ]);
    expect(railDrop(servers, folders, server('c'), server('b'), 'before')).toEqual([
      { op: 'reorderInFolder', ids: ['c', 'b'] }
    ]);
  });

  test('a folder never goes inside another', () => {
    expect(railDrop(servers, folders, folder('g'), server('b'), 'after')).toEqual([]);
  });

  test('leaving a folder of two dissolves it, and the order names the one left in its place', () => {
    expect(railDrop(servers, folders, server('e'), server('a'), 'after')).toEqual([
      { op: 'setFolder', serverId: 'e', folderId: null },
      { op: 'reorder', ordered: [server('a'), server('e'), folder('f'), server('d'), server('h')] }
    ]);
  });

  test('a drop on itself or on something gone does nothing', () => {
    expect(railDrop(servers, folders, server('a'), server('a'), 'after')).toEqual([]);
    expect(railDrop(servers, folders, server('a'), server('zz'), 'after')).toEqual([]);
  });
});
