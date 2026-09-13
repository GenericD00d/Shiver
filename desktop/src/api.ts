import { invoke } from '@tauri-apps/api/core';

import type {
  DmEntry,
  Folder,
  Notification,
  Registry,
  ServerEntry,
  ServerInfo,
  Settings,
  VoiceStatus
} from './types';

/**
 * Every call into the rust core. The core owns the server list, the webviews and the credential
 * store, so the shell never touches a server's origin directly.
 */
export const api = {
  listRegistry: () => invoke<Registry>('list_registry'),

  probeServer: (origin: string) => invoke<ServerInfo>('probe_server', { origin }),

  /**
   * `rememberPassword` is the user's choice about the password, and it is off unless they ask.
   * The session is kept either way — it is what makes a server open straight into the app, and it
   * expires in a week on its own. The password is what would outlive that.
   */
  addServer: (
    origin: string,
    identity?: string,
    password?: string,
    accountLabel?: string,
    rememberPassword = false
  ) =>
    invoke<ServerEntry>('add_server', {
      origin,
      identity: identity || null,
      password: password || null,
      accountLabel: accountLabel || null,
      rememberPassword
    }),

  signInServer: (id: string, identity: string, password: string, rememberPassword = false) =>
    invoke<void>('sign_in_server', { id, identity, password, rememberPassword }),

  /** drops the session and the password, and leaves the server asking who you are */
  logOutServer: (id: string) => invoke<void>('log_out_server', { id }),

  listNotifications: () => invoke<Notification[]>('list_notifications'),

  listDms: () => invoke<DmEntry[]>('list_dms'),

  unreadCount: () => invoke<number>('unread_count'),

  /** unread per rail entry, for the badges on the server icons */
  unreadCounts: () => invoke<Record<string, number>>('unread_counts'),

  /** clears Shiver's badge for one server and marks that server's own channels read */
  markServerRead: (entryId: string) => invoke<void>('mark_server_read', { entryId }),

  markNotificationsRead: () => invoke<void>('mark_notifications_read'),

  clearNotifications: () => invoke<void>('clear_notifications'),

  setChannelMuted: (entryId: string, channelId: number, muted: boolean) =>
    invoke<void>('set_channel_muted', { entryId, channelId, muted }),

  showServerMenu: (id: string) => invoke<void>('show_server_menu', { id }),

  togglePopup: () => invoke<boolean>('toggle_popup'),

  closePopup: () => invoke<void>('close_popup'),

  /** closes the feed because the user clicked away from it, rather than closing it deliberately */
  dismissPopup: () => invoke<void>('dismiss_popup'),

  openDm: (entryId: string, name: string) => invoke<void>('open_dm', { entryId, name }),

  exitDmSplit: () => invoke<void>('exit_dm_split'),

  removeServer: (id: string) => invoke<void>('remove_server', { id }),

  reorderServers: (orderedIds: string[]) => invoke<void>('reorder_servers', { orderedIds }),

  reorderRail: (ordered: { kind: 'server' | 'folder'; id: string }[]) =>
    invoke<void>('reorder_rail', { ordered }),

  createFolderWith: (name: string, memberIds: string[]) =>
    invoke<Folder>('create_folder_with', { name, memberIds }),

  showFolderMenu: (id: string) => invoke<void>('show_folder_menu', { id }),

  setServerFolder: (id: string, folderId: string | null) =>
    invoke<void>('set_server_folder', { id, folderId }),

  createFolder: (name: string) => invoke<Folder>('create_folder', { name }),

  renameFolder: (id: string, name: string) => invoke<void>('rename_folder', { id, name }),

  deleteFolder: (id: string) => invoke<void>('delete_folder', { id }),

  setFolderExpanded: (id: string, expanded: boolean) =>
    invoke<void>('set_folder_expanded', { id, expanded }),

  selectServer: (id: string) => invoke<void>('select_server', { id }),

  /** brings a server up hidden, and reports whether its client is already connected */
  prepareServer: (id: string) => invoke<boolean>('prepare_server', { id }),

  voiceStatus: () => invoke<VoiceStatus | null>('voice_status'),

  /** acts on whichever server holds the call, not on the one being viewed */
  voiceControl: (action: 'mic' | 'sound' | 'leave') => invoke<void>('voice_control', { action }),

  showShell: () => invoke<void>('show_shell'),

  /** which build this is, for the settings screen and for answering "what are you running" */
  /** Forgets the camera and microphone answers WebView2 remembers, for every server. */
  /** forgets the stored camera and microphone answers; resolves with how many there were */
  resetMediaPermissions: () => invoke<number>('reset_media_permissions'),
  appVersion: () => invoke<string>('app_version'),

  getSettings: () => invoke<Settings>('get_settings'),

  updateSettings: (settings: Settings) => invoke<void>('update_settings', { settings }),

  refreshServerInfo: (id: string) => invoke<ServerEntry>('refresh_server_info', { id })
};

/** Tauri rejects with the rust error's user-facing message, which is already worth showing. */
export const errorMessage = (error: unknown) =>
  typeof error === 'string' ? error : error instanceof Error ? error.message : 'Something went wrong';
