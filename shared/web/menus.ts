/**
 * The rail's menus, as one item model both clients draw: the desktop client as a native menu
 * (`show_menu`), the Android client with `components/Menu.tsx`. An item's id is `action:target`,
 * which is what comes back when it is chosen (`readMenuId`).
 */

import type { NotifyLevel } from './types';

export type MenuEntry =
  | { kind: 'item'; id: string; label: string; checked?: boolean; disabled?: boolean }
  | { kind: 'separator' }
  | { kind: 'submenu'; label: string; items: MenuEntry[] };

/** Every action a rail menu can ask for. `move`'s target is `<server id>:<folder id>`. */
export type MenuAction =
  | 'open'
  | 'markread'
  | 'refresh'
  | 'notify-all'
  | 'notify-mentions'
  | 'notify-dms'
  | 'forgetpw'
  | 'plugin'
  | 'anysize'
  | 'normalsize'
  | 'move'
  | 'unfolder'
  | 'logout'
  | 'signin'
  | 'remove'
  | 'rename-folder'
  | 'delete-folder';

const ACTIONS: readonly MenuAction[] = [
  'open',
  'markread',
  'refresh',
  'notify-all',
  'notify-mentions',
  'notify-dms',
  'forgetpw',
  'plugin',
  'anysize',
  'normalsize',
  'move',
  'unfolder',
  'logout',
  'signin',
  'remove',
  'rename-folder',
  'delete-folder'
];

/** A chosen item's action and what it acts on; null for an id no menu here made. */
export const readMenuId = (id: string): { action: MenuAction; target: string } | null => {
  const split = id.indexOf(':');
  const action = ACTIONS.find((candidate) => candidate === id.slice(0, split));

  return split > 0 && action && split < id.length - 1 ? { action, target: id.slice(split + 1) } : null;
};

const item = (action: MenuAction, target: string, label: string, extra?: { checked?: boolean; disabled?: boolean }): MenuEntry => ({
  kind: 'item',
  id: `${action}:${target}`,
  label,
  ...extra
});

const LEVELS: [MenuAction, NotifyLevel, string][] = [
  ['notify-all', 'all', 'All messages'],
  ['notify-mentions', 'mentions', 'Mentions and direct messages'],
  ['notify-dms', 'dms', 'Direct messages only']
];

export type MenuServer = {
  id: string;
  /** signed in through Shiver, so there is a session to log out of */
  identity: string | null;
  folderId: string | null;
  notify: NotifyLevel;
  acceptAnySize?: boolean;
};

/** What the menu says about a server beyond its entry; what a platform does not know, it leaves out. */
export type ServerMenuFacts = {
  folders: readonly { id: string; name: string }[];
  /** whether Shiver keeps its password; "Forget my password" is offered only then */
  hasPassword: boolean;
  /** the companion plugin's version, null where it is not installed, undefined before Shiver has looked */
  plugin?: string | null;
  /** it has sent a message over the size limit, so raising the limit is worth offering */
  tooLarge: boolean;
  /** the platform can mark a server's channels read from outside its page */
  canMarkRead: boolean;
};

const pluginLabel = (plugin: string | null | undefined) =>
  plugin === undefined ? 'Shiver plugin: not checked yet' : plugin ? `✓ Shiver plugin ${plugin}` : '✗ No Shiver plugin';

/** A server's menu, in the order both clients show it. */
export const serverMenu = (server: MenuServer, facts: ServerMenuFacts): MenuEntry[] => {
  const { id } = server;
  const entries: MenuEntry[] = [item('open', id, 'Open')];

  if (facts.canMarkRead) entries.push(item('markread', id, 'Mark all as read'));

  entries.push(
    item('refresh', id, 'Refresh name and icon'),
    {
      kind: 'submenu',
      label: 'Notify me about',
      items: LEVELS.map(([action, level, label]) => item(action, id, label, { checked: server.notify === level }))
    },
    item('forgetpw', id, 'Forget my password', { disabled: !facts.hasPassword }),
    item('plugin', id, pluginLabel(facts.plugin), { disabled: true })
  );

  // the size limit is only offered where it matters: a server that tripped it, or one raised
  if (server.acceptAnySize) entries.push(item('normalsize', id, 'Back to the normal size limit'));
  else if (facts.tooLarge) entries.push(item('anysize', id, 'Accept larger messages from this server'));

  const moves = facts.folders
    .filter((folder) => folder.id !== server.folderId)
    .map((folder) => item('move', `${id}:${folder.id}`, `Move to “${folder.name}”`));

  if (moves.length > 0 || server.folderId) {
    entries.push({ kind: 'separator' }, ...moves);

    if (server.folderId) entries.push(item('unfolder', id, 'Move out of folder'));
  }

  entries.push(
    { kind: 'separator' },
    server.identity ? item('logout', id, 'Log out') : item('signin', id, 'Sign in'),
    item('remove', id, 'Remove from Shiver')
  );

  return entries;
};

/** A folder's menu. Its servers are not deleted with it; they go back to the top level. */
export const folderMenu = (folderId: string): MenuEntry[] => [
  item('rename-folder', folderId, 'Rename folder'),
  { kind: 'separator' },
  item('delete-folder', folderId, 'Delete folder')
];
