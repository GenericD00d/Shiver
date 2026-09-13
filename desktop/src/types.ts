/** Sharkord's own dark theme. Must match `DEFAULT_THEME_COLOR` / `DEFAULT_ACCENT_COLOR` in model.rs. */
export const DEFAULT_THEME_COLOR = '#0a0a0a';
export const DEFAULT_ACCENT_COLOR = '#e5e5e5';

/** Sharkord's `--sidebar`, used for the rail while the user is on the default colours. */
export const DEFAULT_RAIL_COLOR = '#171717';

/** Loudest the sound slider goes. Must match `MAX_SOUND_VOLUME` in model.rs, which clamps it. */
export const MAX_SOUND_VOLUME = 250;

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

export type MutedChannel = {
  entryId: string;
  channelId: number;
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

export type Folder = {
  id: string;
  name: string;
  position: number;
  expanded: boolean;
};

/**
 * The call Shiver is showing in the rail.
 *
 * There is at most one across every server, which is the rule DESIGN.md asks for: `entryId` says
 * which server is holding it, and Shiver's controls act there rather than on whatever is on screen.
 */
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
};

export type Registry = {
  servers: ServerEntry[];
  folders: Folder[];
  settings: Settings;
  muted: MutedChannel[];
};

export type ServerInfo = {
  origin: string;
  serverId: string;
  name: string;
  description: string | null;
  iconUrl: string | null;
};
