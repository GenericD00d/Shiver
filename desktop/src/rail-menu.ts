/**
 * The rail's menus: drawn as native menus from the item model both clients share (`menus.ts`),
 * with the item chosen coming back as an event and carried out here.
 */

import { useCallback } from 'react';

import { api } from './api';
import { EVENTS, useCoreEvent } from './events';
import type { Registry } from './types';
import { folderMenu, notifyLevelOf, readMenuId, serverMenu } from '../../shared/web/menus';
import type { RailRef } from '../../shared/web/rail';

/** A screen about one server or folder, which the menu opens. */
export type AskPanel = 'remove' | 'signin' | 'folder';

type Setup = {
  registry: Registry;
  refresh: () => Promise<unknown>;
  refreshFeed: () => Promise<unknown>;
  openServer: (id: string) => Promise<unknown>;
  /** opens the screen about `id` */
  ask: (panel: AskPanel, id: string) => Promise<unknown>;
};

/** Carries out the item chosen in a rail menu, and returns what shows one. */
export function useRailMenu({ registry, refresh, refreshFeed, openServer, ask }: Setup) {
  useCoreEvent<string>(EVENTS.menu, (id) => {
    const chosen = readMenuId(id);

    if (!chosen) return;

    const { action, target } = chosen;
    const notify = () => api.setNotifyLevel(target, notifyLevelOf(action) ?? 'all').then(refresh);
    const actions: Record<typeof action, () => Promise<unknown>> = {
      open: () => openServer(target),
      remove: () => ask('remove', target),
      signin: () => ask('signin', target),
      'rename-folder': () => ask('folder', target),
      markread: () => api.markServerRead(target).then(refreshFeed),
      forgetpw: () => api.forgetPassword(target),
      // two actions rather than a toggle: the menu knew which way the server is set
      anysize: () => api.setAcceptAnySize(target, true),
      normalsize: () => api.setAcceptAnySize(target, false),
      'notify-all': notify,
      'notify-mentions': notify,
      'notify-dms': notify,
      logout: () => api.logOutServer(target).then(refresh),
      refresh: () => api.refreshServerInfo(target).then(refresh),
      move: () => {
        const [serverId, folderId] = target.split(':');

        return api.setServerFolder(serverId, folderId).then(refresh);
      },
      unfolder: () => api.setServerFolder(target, null).then(refresh),
      'delete-folder': () => api.deleteFolder(target).then(refresh),
      plugin: async () => undefined
    };

    void actions[action]().catch(() => undefined);
  });

  /** A server's or folder's menu, from the item model both clients share. */
  return useCallback(
    async (target: RailRef) => {
      if (target.kind === 'folder') {
        await api.showMenu(folderMenu(target.id));

        return;
      }

      const server = registry.servers.find((candidate) => candidate.id === target.id);

      if (!server) return;

      const facts = await api.serverMenuFacts(server.id);

      await api.showMenu(
        serverMenu(server, {
          folders: registry.folders,
          hasPassword: facts.hasPassword,
          plugin: facts.pluginChecked ? facts.plugin : undefined,
          tooLarge: facts.tooLarge,
          canMarkRead: true
        })
      );
    },
    [registry.folders, registry.servers]
  );
}
