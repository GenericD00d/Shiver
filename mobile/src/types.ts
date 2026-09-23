export { DEFAULT_ACCENT_COLOR, DEFAULT_RAIL_COLOR, DEFAULT_THEME_COLOR, MAX_SOUND_VOLUME } from '../../shared/web/settings';

export type ServerEntry = {
  /** Shiver takes messages of any size from this server, because the user said it may */
  acceptAnySize?: boolean;
  id: string;
  origin: string;
  serverId: string | null;
  name: string;
  iconUrl: string | null;
  /** the logo as a `data:` uri, sent to the bridge so a server's page never learns another's address */
  iconData: string | null;
  identity: string | null;
  accountLabel: string | null;
  folderId: string | null;
  position: number;
};

/**
 * One conversation, and which server and account it belongs to.
 *
 * Only Shiver's own screens ever see this. A server's page is handed no conversation list at all —
 * it draws its own from Sharkord's store — so that one server never learns who the user privately
 * messages on the others. Mirrors `DmEntry` in `inbox.rs`.
 */
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
  registered: number;
  failed: number;
  /** every server, with whether it was chosen to wake the phone and how that is going */
  servers: PushServer[];
};

export type PushServer = {
  id: string;
  name: string;
  wanted: boolean;
  /** `off`, `waiting` for the distributor to answer, `ready`, or `failed` */
  state: 'off' | 'waiting' | 'ready' | 'failed';
};

/** A server Shiver cannot watch, and the reason in words meant for a person. */
/** A server looked up before it is added, with the companion plugin's verdict where one is possible. */
export type ServerCheck = ServerInfo & {
  /**
   * The plugin's version, null where the server has none — and **absent entirely** when Shiver
   * could not ask, which is the case without credentials. Nothing reveals a server's plugins
   * without a session, so an unasked server must not be drawn as lacking it.
   */
  plugin?: string | null;
};

/** What Shiver found out about a server's companion plugin, once it had connected to it. */
export type PluginStatus = {
  entryId: string;
  /** null where Shiver connected and the plugin was not installed */
  version: string | null;
};

export type WatchProblem = {
  entryId: string;
  reason: string;
};

export type MutedChannel = {
  entryId: string;
  channelId: number;
};

export type Folder = {
  id: string;
  name: string;
  position: number;
  expanded: boolean;
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
  /**
   * Which servers may wake the phone while Shiver is closed, by rail entry id.
   *
   * Owned by the background-notifications screen, not the settings one — but declared here because
   * saving settings sends this whole object back, and a field missing from the object is a field
   * cleared in the registry.
   */
  pushServers: string[];
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
  /** the logo as a `data:` uri, sent to the bridge so a server's page never learns another's address */
  iconData: string | null;
};
