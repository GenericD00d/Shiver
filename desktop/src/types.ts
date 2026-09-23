import type { Folder, MutedChannel, ServerCheck, ServerInfo } from '../../shared/web/types';

export { DEFAULT_ACCENT_COLOR, DEFAULT_RAIL_COLOR, DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME } from '../../shared/web/settings';

export type { Folder, MutedChannel, ServerCheck, ServerInfo };

export type ServerEntry = {
  id: string;
  origin: string;
  serverId: string | null;
  name: string;
  iconUrl: string | null;
  identity: string | null;
  accountLabel: string | null;
  folderId: string | null;
  position: number;
};

export type Notification = {
  id: number;
  entryId: string;
  serverName: string;
  channelId: number | null;
  channelName: string | null;
  author: string;
  body: string;
  iconUrl: string | null;
  isDm: boolean;
  at: number;
  read: boolean;
  /** the version on offer, when this entry is Shiver rather than a server */
  update: string | null;
};

export type DmChannel = {
  channelId: number;
  name: string;
  iconUrl: string | null;
  /** null until Shiver sees a message here: the plugin store exposes channels, not their messages */
  lastMessageAt: number | null;
};

export type DmEntry = {
  entryId: string;
  serverName: string;
  accountLabel: string;
  channel: DmChannel;
};

/** What the badge on a server's icon says. */
export type ServerStatus = 'online' | 'connecting' | 'offline';

/** The one call across all servers; controls act on `entryId`, not on the server on screen. */
export type VoiceStatus = {
  entryId: string;
  serverName: string;
  accountLabel: string;
  channelId: number;
  channelName: string | null;
  micMuted: boolean;
  soundMuted: boolean;
  /** Sharkord disables its own mic control while deafened, and Shiver's follows */
  micLocked: boolean;
};

export type Settings = {
  themeColor: string;
  accentColor: string;
  /** the colour text is drawn in, or null to take it from the background */
  textColor: string | null;
  notificationSounds: boolean;
  /**
   * How loud Shiver's own ping and each server's own sounds are, as a percentage of what they would
   * otherwise be. Goes past 100: see `sound_volume` in model.rs for why that is possible at all.
   */
  soundVolume: number;
  /** shrink the file card under a picture down to its icon; see `minimise_attachments` in model.rs */
  minimiseAttachments: boolean;
  lastServerId: string | null;
  /** a system-wide shortcut that mutes the microphone, in Tauri's accelerator form */
  muteHotkey: string | null;
  /** how many servers keep a live page rather than a socket; see `pages_kept` in model.rs */
  pagesKept: number;
};

/** Both ends of `pagesKept`. Kept in step with `MIN_PAGES_KEPT` / `MAX_PAGES_KEPT` in model.rs,
 *  which clamps whatever arrives — this pair only decides what the slider will let you ask for. */
export const MIN_PAGES_KEPT = 1;
export const MAX_PAGES_KEPT = 20;
export const DEFAULT_PAGES_KEPT = 3;

export type Registry = {
  servers: ServerEntry[];
  folders: Folder[];
  settings: Settings;
  muted: MutedChannel[];
};
