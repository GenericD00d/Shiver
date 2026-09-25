import type { ShiverTheme } from '../../shared/web/bridge/theme';

export type ShiverConfig = {
  entryId: string;
  origin: string;
  serverName: string;
  /** null while the user is on Sharkord's own colours */
  theme: ShiverTheme | null;
  muted: number[];
  /** Shiver's own page, which back and the swipe past the drawer navigate to (there is no IPC) */
  home: string;
  /** open the DM with this person on arrival */
  openDmUser: string | null;
  /** endpoints this server's plugin should forget (push was turned off for it) */
  retiredPushEndpoints: string[];
  /** this server's own session, used to reconnect; never another server's */
  session: string | null;
  /** this server's per-channel unread floor to store in the companion plugin */
  readFloor?: Record<string, number> | null;
  /** this entry's own UnifiedPush endpoint, for the plugin's relay */
  pushEndpoint: string | null;
  minimiseAttachments: boolean;
  /** percentage of Sharkord's own sound level */
  soundVolume: number;
};

declare global {
  interface Window {
    __SHIVER__?: ShiverConfig;
    __SHIVER_MOBILE_INSTALLED__?: boolean;
    /** polled by the core: this server's muted channels */
    __SHIVER_MUTED__?: () => number[];
    /** polled by the core: links to open in the browser (handed over once) */
    __SHIVER_OPEN__?: () => string[];
    /** asked by the activity on a back press; true when the page used it */
    __SHIVER_BACK__?: () => boolean;
  }
}
