import { describe, expect, test } from 'bun:test';

import { folderMenu, type MenuEntry, readMenuId, serverMenu } from './menus';

const ids = (entries: MenuEntry[]): string[] =>
  entries.flatMap((entry) => (entry.kind === 'item' ? [entry.id] : entry.kind === 'submenu' ? ids(entry.items) : ['---']));

const facts = { folders: [{ id: 'f', name: 'Games' }], hasPassword: true, plugin: '0.2.0', tooLarge: false, canMarkRead: true };

describe('rail menus', () => {
  test("a server's menu, in order", () => {
    expect(ids(serverMenu({ id: 's', identity: 'me', folderId: null, notify: 'all' }, facts))).toEqual([
      'open:s',
      'markread:s',
      'refresh:s',
      'notify-all:s',
      'notify-mentions:s',
      'notify-dms:s',
      'forgetpw:s',
      'plugin:s',
      '---',
      'move:s:f',
      '---',
      'logout:s',
      'remove:s'
    ]);
  });

  test('a platform hides what it lacks, and the menu follows the server', () => {
    const entries = serverMenu(
      { id: 's', identity: null, folderId: 'f', notify: 'dms', acceptAnySize: true },
      { ...facts, canMarkRead: false, hasPassword: false, plugin: undefined }
    );

    expect(ids(entries)).toEqual([
      'open:s',
      'refresh:s',
      'notify-all:s',
      'notify-mentions:s',
      'notify-dms:s',
      'forgetpw:s',
      'plugin:s',
      'normalsize:s',
      '---',
      'unfolder:s',
      '---',
      'signin:s',
      'remove:s'
    ]);

    const levels = entries.find((entry) => entry.kind === 'submenu');

    expect(levels?.kind === 'submenu' && levels.items.map((entry) => entry.kind === 'item' && entry.checked)).toEqual([
      false,
      false,
      true
    ]);
    expect(entries.find((entry) => entry.kind === 'item' && entry.id === 'forgetpw:s')).toMatchObject({ disabled: true });
  });

  test('a server that sent too much is offered the larger limit', () => {
    expect(ids(serverMenu({ id: 's', identity: 'me', folderId: null, notify: 'all' }, { ...facts, tooLarge: true }))).toContain(
      'anysize:s'
    );
  });

  test('chosen ids read back, and nothing else does', () => {
    expect(readMenuId('move:s:f')).toEqual({ action: 'move', target: 's:f' });
    expect(ids(folderMenu('f')).map((id) => readMenuId(id)?.action ?? id)).toEqual(['rename-folder', '---', 'delete-folder']);
    expect(readMenuId('quit')).toBeNull();
    expect(readMenuId('open:')).toBeNull();
    expect(readMenuId('toString:s')).toBeNull();
  });
});
