import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type {
  DmEntry,
  Folder,
  PluginStatus,
  PushStatus,
  ServerCheck,
  WatchProblem,
  Registry,
  ServerEntry,
  ServerInfo,
  Settings
} from './types';

/** Calls into the core. Only Shiver's own pages have IPC; server pages cannot make these. */
export const api = {
  listRegistry: () => invoke<Registry>('list_registry'),

  probeServer: (origin: string) => invoke<ServerInfo>('probe_server', { origin }),
  /** looks a server up and, when credentials are given, says whether it has the Shiver plugin */
  checkServer: (origin: string, identity: string | null, password: string | null) =>
    invoke<ServerCheck>('check_server', { origin, identity, password }),

  /** identity and password are optional: without them the user signs in on the server's own page */
  addServer: (
    origin: string,
    identity: string | null,
    password: string | null,
    rememberPassword = false
  ) => invoke<ServerEntry>('add_server', { origin, identity, password, rememberPassword }),

  /** Signs an existing server in again (sessions last a week and cannot be refreshed). */
  signInServer: (id: string, identity: string, password: string, rememberPassword: boolean) =>
    invoke<void>('sign_in_server', { id, identity, password, rememberPassword }),

  /** servers whose session expired and that Shiver cannot renew on its own */
  signedOutServers: () => invoke<string[]>('signed_out_servers'),

  /** servers Shiver can sign in again by itself, because the user asked it to keep the password */
  rememberedServers: () => invoke<string[]>('remembered_servers'),

  removeServer: (id: string) => invoke<void>('remove_server', { id }),

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
  selectServer: (id: string, dms = false, dmUser?: string) =>
    invoke<void>('select_server', { id, dms, dmUser }),



  /** which build this is, for the settings screen and for answering "what are you running" */
  appVersion: () => invoke<string>('app_version'),

  /** the newer version, when the core has found one; null otherwise */
  /** takes back the stored password for one server; the session is left alone */
  forgetPassword: (id: string) => invoke<void>('forget_password', { id }),
  updateAvailable: () => invoke<string | null>('update_available'),
  /** turn one version down, so nothing mentions it again — not even on the next launch */
  skipUpdate: (version: string) => invoke<void>('skip_update', { version }),
  /** opens the project's page in the browser */
  openRepository: () => invoke<void>('open_repository'),
  /** asks GitHub now; null means nothing newer. Ignores a skipped version — see `check_for_update` */
  checkForUpdate: () => invoke<string | null>('check_for_update'),
  /** opens the releases page in the browser, which is where the apk actually comes from */
  openReleases: () => invoke<void>('open_releases'),

  updateSettings: (settings: Settings) => invoke<void>('update_settings', { settings }),

  /** re-reads a server's name and logo from its public /info */
  refreshServerInfo: (id: string) => invoke<ServerEntry>('refresh_server_info', { id }),

  /** both refuse unless that server is the one on screen; only its own client can do them */

  logOutServer: (id: string) => invoke<void>('log_out_server', { id }),

  /** Forgets every session and password Shiver holds; pages' own sign-ins are left alone. */
  forgetSessions: () => invoke<void>('forget_sessions'),

  /** unread per server, counted by the core over its own connections */
  unreadCounts: () => invoke<Record<string, number>>('unread_counts'),

  /** Every server's conversations; only Shiver's own pages can see them. */
  listDms: () => invoke<DmEntry[]>('list_dms'),

  /** distributors installed, and how many servers can be woken through the chosen one */
  watchProblems: () => invoke<WatchProblem[]>('watch_problems'),
  serverPlugins: () => invoke<PluginStatus[]>('server_plugins'),
  setAcceptAnySize: (id: string, accept: boolean) =>
    invoke<void>('set_accept_any_size', { id, accept }),
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
