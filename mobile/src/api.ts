import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type {
  DmEntry,
  Folder,
  PluginStatus,
  PushStatus,
  WatchProblem,
  Registry,
  ServerEntry,
  ServerInfo,
  Settings
} from './types';

/**
 * Every call into the rust core.
 *
 * Only Shiver's own pages can make these. A server's page is on its own origin and has no Tauri IPC,
 * which is what stops a server reaching anything Shiver knows — the same rule as on desktop.
 */
export const api = {
  listRegistry: () => invoke<Registry>('list_registry'),

  probeServer: (origin: string) => invoke<ServerInfo>('probe_server', { origin }),

  /** identity and password are optional: without them the user signs in on the server's own page */
  addServer: (
    origin: string,
    identity: string | null,
    password: string | null,
    rememberPassword = false
  ) => invoke<ServerEntry>('add_server', { origin, identity, password, rememberPassword }),

  /**
   * Signs an existing server in again.
   *
   * The way back from a session Shiver could not renew: Sharkord's last a week and cannot be
   * refreshed, so a server Shiver holds no password for eventually needs this once.
   */
  signInServer: (id: string, identity: string, password: string, rememberPassword: boolean) =>
    invoke<void>('sign_in_server', { id, identity, password, rememberPassword }),

  /** servers whose session expired and that Shiver cannot renew on its own */
  signedOutServers: () => invoke<string[]>('signed_out_servers'),

  /** servers Shiver can sign in again by itself, because the user asked it to keep the password */
  rememberedServers: () => invoke<string[]>('remembered_servers'),

  removeServer: (id: string) => invoke<void>('remove_server', { id }),

  /** hands the one webview over to that server's client; `dms` asks it to open its dm list */
  reorderServers: (orderedIds: string[]) => invoke<void>('reorder_servers', { orderedIds }),
  reorderRail: (ordered: { kind: 'server' | 'folder'; id: string }[]) =>
    invoke<void>('reorder_rail', { ordered }),
  createFolderWith: (name: string, memberIds: string[]) =>
    invoke<Folder>('create_folder_with', { name, memberIds }),
  setServerFolder: (id: string, folderId: string | null) =>
    invoke<void>('set_server_folder', { id, folderId }),
  renameFolder: (id: string, name: string) => invoke<void>('rename_folder', { id, name }),
  deleteFolder: (id: string) => invoke<void>('delete_folder', { id }),
  setFolderExpanded: (id: string, expanded: boolean) =>
    invoke<void>('set_folder_expanded', { id, expanded }),
  openServer: (id: string, dms = false, dmUser?: string) =>
    invoke<void>('open_server', { id, dms, dmUser }),

  showShiver: () => invoke<void>('show_shiver'),

  showingServer: () => invoke<string | null>('showing_server'),

  /** which build this is, for the settings screen and for answering "what are you running" */
  appVersion: () => invoke<string>('app_version'),

  /** the newer version, when the core has found one; null otherwise */
  /** takes back the stored password for one server; the session is left alone */
  forgetPassword: (id: string) => invoke<void>('forget_password', { id }),
  updateAvailable: () => invoke<string | null>('update_available'),
  /** opens the releases page in the browser, which is where the apk actually comes from */
  openReleases: () => invoke<void>('open_releases'),
  getSettings: () => invoke<Settings>('get_settings'),

  updateSettings: (settings: Settings) => invoke<void>('update_settings', { settings }),

  /** re-reads a server's name and logo from its public /info */
  refreshServerInfo: (id: string) => invoke<ServerEntry>('refresh_server_info', { id }),

  /** both refuse unless that server is the one on screen; only its own client can do them */
  markServerRead: (id: string) => invoke<void>('mark_server_read', { id }),

  logOutServer: (id: string) => invoke<void>('log_out_server', { id }),

  /**
   * Drops every session and password Shiver is holding, for every server.
   *
   * The way to say no to Shiver watching servers in the background. Each server's own client keeps
   * its own sign-in, which this leaves alone — that one is the user's, set up on the server's page.
   */
  forgetSessions: () => invoke<void>('forget_sessions'),

  /** unread per server, counted by the core over its own connections */
  listUnread: () => invoke<Record<string, number>>('list_unread'),

  /**
   * Every server's conversations, gathered in one list.
   *
   * Lives here rather than in the rail a server's page draws, because only Shiver's own pages have
   * IPC — which is what stops one server reading who the user messages on all the others. The
   * server currently on screen is absent: its own page shows its conversations itself.
   */
  listDms: () => invoke<DmEntry[]>('list_dms'),

  /** distributors installed, and how many servers can be woken through the chosen one */
  watchProblems: () => invoke<WatchProblem[]>('watch_problems'),
  serverPlugins: () => invoke<PluginStatus[]>('server_plugins'),
  setServerAcceptsAnySize: (entryId: string, accept: boolean) =>
    invoke<void>('set_server_accepts_any_size', { entryId, accept }),
  pushStatus: () => invoke<PushStatus>('push_status'),
  setPushServer: (entryId: string, wanted: boolean) =>
    invoke<void>('set_push_server', { entryId, wanted }),

  /** picks a distributor and asks it for an endpoint per server; the endpoints arrive later */
  setPushDistributor: (distributor: string) =>
    invoke<void>('set_push_distributor', { distributor }),

  /** fires when an endpoint arrives or a registration is refused, so the screen can redraw */
  onPush: (handler: () => void) => listen('shiver://push', () => handler()),

  /** fires whenever one of those counts moves */
  onUnread: (handler: (unread: Record<string, number>) => void) =>
    listen<Record<string, number>>('shiver://inbox', (event) => handler(event.payload))
};

/** Tauri rejects with the rust error's user-facing message, which is already worth showing. */
export const errorMessage = (error: unknown) =>
  typeof error === 'string' ? error : error instanceof Error ? error.message : 'Something went wrong';
