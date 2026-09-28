/** Types both clients receive from their cores alike. */

export type MutedChannel = {
  entryId: string;
  channelId: number;
};

/** Which of a server's messages notify (`model::NotifyLevel`); unread badges count them all. */
export type NotifyLevel = 'all' | 'mentions' | 'dms';

/** A site whose links open without asking, when this rail entry's pages ask (`links::TrustedLink`). */
export type TrustedLink = {
  entryId: string;
  site: string;
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
