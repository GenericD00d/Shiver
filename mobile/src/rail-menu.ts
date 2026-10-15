/**
 * The rail's menus on Android, for Shiver's page and for the screens over a server's page alike:
 * what a tile's menu lists, and what each choice does, through the actions each of them supplies.
 */

import { api } from './api';
import type { ConfirmAction } from './components/ActionConfirm';
import type { Folder, ServerEntry, SessionStates } from './types';
import { folderMenu, type MenuEntry, notifyLevelOf, readMenuId, serverMenu } from '../../shared/web/menus';
import type { RailRef } from '../../shared/web/rail';

/** A server's or folder's menu, from the item model both clients share; null for one unknown. */
export const railMenu = (
  target: RailRef,
  servers: readonly ServerEntry[],
  folders: readonly Folder[],
  sessions: SessionStates
): MenuEntry[] | null => {
  if (target.kind === 'folder') return folderMenu(target.id);

  const server = servers.find((candidate) => candidate.id === target.id);

  if (!server) return null;

  return serverMenu(server, {
    folders,
    hasPassword: sessions.remembered.includes(server.id),
    plugin: server.id in sessions.plugins ? sessions.plugins[server.id] : undefined,
    // a problem watching it is what the size limit raises; the settings list offers the same
    tooLarge: server.id in sessions.problems,
    canMarkRead: false
  });
};

/** What a menu choice needs from the screen it was made on. */
export type MenuActions = {
  open: (entryId: string) => void;
  /** runs a change, then reads everything again */
  change: (run: () => Promise<unknown>) => Promise<void>;
  /** asks before an action that cannot be taken back */
  ask: (action: ConfirmAction, entryId: string) => void;
  signIn: (entryId: string) => void;
  rename: (folderId: string) => void;
  /** a server's logo may have changed */
  iconsChanged: () => void;
};

/** Does what a menu item (`menus.ts` id) asks. */
export const chooseMenuItem = (id: string, actions: MenuActions) => {
  const chosen = readMenuId(id);

  if (!chosen) return;

  const { action, target } = chosen;

  switch (action) {
    case 'open':
      actions.open(target);
      break;
    case 'refresh':
      void actions.change(() => api.refreshServerInfo(target)).then(actions.iconsChanged);
      break;
    case 'notify-all':
    case 'notify-mentions':
    case 'notify-dms':
      void actions.change(() => api.setNotifyLevel(target, notifyLevelOf(action) ?? 'all'));
      break;
    case 'anysize':
    case 'normalsize':
      void actions.change(() => api.setAcceptAnySize(target, action === 'anysize'));
      break;
    case 'forgetpw':
    case 'logout':
    case 'remove':
      actions.ask(action, target);
      break;
    case 'signin':
      actions.signIn(target);
      break;
    case 'move': {
      const [serverId, folderId] = target.split(':');

      void actions.change(() => api.setServerFolder(serverId, folderId));
      break;
    }
    case 'unfolder':
      void actions.change(() => api.setServerFolder(target, null));
      break;
    case 'rename-folder':
      actions.rename(target);
      break;
    case 'delete-folder':
      void actions.change(() => api.deleteFolder(target));
      break;
    case 'markread':
    case 'plugin':
      break;
  }
};

/** Runs a confirmed action. */
export const runConfirmed = (action: ConfirmAction, entryId: string) =>
  ({ remove: api.removeServer, forgetpw: api.forgetPassword, logout: api.logOutServer })[action](entryId);
