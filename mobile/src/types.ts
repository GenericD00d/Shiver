import type { Folder, MutedChannel, ServerCheck, ServerInfo } from '../../shared/web/types';

export { DEFAULT_ACCENT_COLOR, DEFAULT_RAIL_COLOR, DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME } from '../../shared/web/settings';

export type { Folder, MutedChannel, ServerCheck, ServerInfo };

export type ServerEntry = {
  /** Shiver takes messages of any size from this server, because the user said it may */
  acceptAnySize?: boolean;
  id: string;
  origin: string;
  name: string;
  /** the stored logo as a `data:` uri, read separately (`api.serverIcons`) */
  icon?: string;
  identity: string | null;
  accountLabel: string | null;
  folderId: string | null;
  position: number;
};

/** One conversation and its server and account (mirrors `DmEntry` in `inbox.rs`); never sent to server pages. */
export type DmEntry = {
  entryId: string;
  serverName: string;
  accountLabel: string | null;
  channelId: number;
  userName: string;
  /**
   * When the last message arrived, in milliseconds, as the server itself reports it — and what the
   * list is ordered by. Null only from a server that answered without the field, which sorts last.
   */
  lastMessageAt: number | null;
};

/** What Shiver knows about being woken while it is closed. Carries no endpoint: see `push_status`. */
export type PushStatus = {
  /** package names of the UnifiedPush distributors installed on this phone */
  distributors: string[];
  chosen: string | null;
  /** every server, with whether it was chosen to wake the phone and how that is going */
  servers: PushServer[];
};

type PushServer = {
  id: string;
  name: string;
  wanted: boolean;
  /** `off`, `waiting` for the distributor to answer, `ready`, or `failed` */
  state: 'off' | 'waiting' | 'ready' | 'failed';
};

/** `SessionStates` in commands.rs: what Shiver's screens show about each server's session. */
export type SessionStates = {
  signedOut: string[];
  /** servers Shiver can sign in again by itself, because it keeps the password */
  remembered: string[];
  /** entry id -> a problem watching it that will not fix itself, in words meant for a person */
  problems: Record<string, string>;
  /** entry id -> plugin version, null where connected without one; absent until connected */
  plugins: Record<string, string | null>;
};

export type Settings = {
  themeColor: string;
  accentColor: string;
  /** the colour text is drawn in, or null to take it from the background */
  textColor: string | null;
  /**
   * How loud each server's own sounds are, as a percentage of what they would otherwise be. Goes
   * past 100: see `sound_volume` in model.rs for why that is possible at all.
   */
  soundVolume: number;
  /** shrink the file card under a picture down to its icon; see `minimise_attachments` in model.rs */
  minimiseAttachments: boolean;
  lastServerId: string | null;
  /** entries allowed to wake the phone while Shiver is closed; kept here because settings are saved whole */
  pushServers: string[];
};

export type Registry = {
  servers: ServerEntry[];
  folders: Folder[];
  settings: Settings;
  muted: MutedChannel[];
};
