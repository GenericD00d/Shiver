/** Types both clients receive from their cores alike. */

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

/** What `/info` says about a server (`probe::ServerInfo`). */
export type ServerInfo = {
  origin: string;
  name: string;
  description: string | null;
  iconUrl: string | null;
};

/**
 * `/info` plus the companion plugin's version: null when it is not installed, absent when Shiver
 * could not ask (no credentials), which must not be drawn as lacking it.
 */
export type ServerCheck = ServerInfo & { plugin?: string | null };
