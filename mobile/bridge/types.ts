import type { ShiverTheme } from '../../shared/web/bridge/theme';
import type { Folder } from '../../shared/web/types';

/** One server as a server's page may know it: no origin, icon URL, account or identity. */
export type RailEntry = {
  id: string;
  name: string;
  /** the logo as a `data:` URI, or null for initials */
  icon: string | null;
  unread: number;
  /** Shiver's session for it expired and cannot be renewed */
  signedOut: boolean;
  /** position among the top level, or within its folder */
  position: number;
  folderId: string | null;
};

export type RailFolder = Folder;

export type RailMove = { serverId: string; folderId: string | null };

export type RailCreate = { id: string; name: string; memberIds: string[] };

export type ShiverConfig = {
  entryId: string;
  origin: string;
  serverName: string;
  /** null while the user is on Sharkord's own colours */
  theme: ShiverTheme | null;
  muted: number[];
  /** Shiver's own page, which the rail navigates to (there is no IPC) */
  home: string;
  rail: RailEntry[];
  folders: RailFolder[];
  /** open Sharkord's DM list on arrival */
  openDms: boolean;
  /** open the DM with this person on arrival */
  openDmUser: string | null;
  /** this server's own session, used to reconnect; never another server's */
  session: string | null;
  /** this server's per-channel unread floor to store in the companion plugin */
  readFloor?: Record<string, number> | null;
  /** this entry's own UnifiedPush endpoint, for the plugin's relay */
  pushEndpoint: string | null;
  /** the settings and drafts kept when this origin's storage was last wiped, as JSON */
  carried: string | null;
  minimiseAttachments: boolean;
  /** percentage of Sharkord's own sound level */
  soundVolume: number;
};

declare global {
  interface Window {
    __SHIVER__?: ShiverConfig;
    __SHIVER_MOBILE_INSTALLED__?: boolean;
    /** the core pushes unread counts in */
    __SHIVER_UNREAD__?: (unread: Record<string, number>) => void;
    /** the core calls this before navigating away */
    __SHIVER_FORGET_SESSION__?: () => void;
    /** polled by the core: this server's muted channels */
    __SHIVER_MUTED__?: () => number[];
    /** polled by the core: links to open in the browser (handed over once) */
    __SHIVER_OPEN__?: () => string[];
    /** polled by the core: the rail's top-level order, and moves and folders made since last asked */
    __SHIVER_RAIL_STATE__?: () => {
      order: { kind: 'server' | 'folder'; id: string }[];
      moves: RailMove[];
      creates: RailCreate[];
    };
    /** asked by the activity on a back press; true when the rail used it */
    __SHIVER_BACK__?: () => boolean;
  }
}
