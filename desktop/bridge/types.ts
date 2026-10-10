/** The desktop bridge's config, what it queues for the core, and the hooks it puts on `window`. */

import type { DmChannel } from '../../shared/web/bridge/sharkord';
import type { ShiverTheme } from '../../shared/web/bridge/theme';
import type { NotifyLevel } from '../../shared/web/types';

export type ShiverConfig = {
  entryId: string;
  origin: string;
  /** null while the user is on Sharkord's own colours */
  theme: ShiverTheme | null;
  /** the session Shiver signed in with; served to the page from memory, never stored */
  token: string | null;
  muted: number[];
  /** the page was opened to show this conversation beside Shiver's DM list */
  conversation: string | null;
  /** another server holds the voice session (which one is never said), so joins are refused */
  voiceLocked: boolean;
  minimiseAttachments: boolean;
  /** percentage of Sharkord's own sound level */
  soundVolume: number;
  /** which of this server's messages notify */
  notify: NotifyLevel;
};

export type QueuedNotification = {
  channelId: number | null;
  channelName: string | null;
  author: string;
  body: string;
  iconUrl: string | null;
  isDm: boolean;
};

export type QueuedMute = { channelId: number; muted: boolean };

/** The voice session on this server: channel from the store, mute flags read off Sharkord's controls. */
export type VoiceSnapshot = {
  channelId: number;
  channelName: string | null;
  micMuted: boolean;
  soundMuted: boolean;
  /** Sharkord disables its mic button while deafened */
  micLocked: boolean;
};

export type VoiceAction = 'mic' | 'sound' | 'leave';

declare global {
  interface Window {
    __SHIVER__?: ShiverConfig;
    /** everything queued since the last call; null when nothing changed, unless `full` (the core drains it on a timer) */
    __SHIVER_DRAIN__?: (full: boolean) => {
      notifications: QueuedNotification[];
      dms: DmChannel[] | null;
      mutes: QueuedMute[];
      /** "Mark all as read" was chosen in the channel menu, which marked Sharkord's side already */
      markAllRead: boolean;
      /** the plugin's reconciled mute list, sent once */
      syncedMutes: number[] | null;
      /** the conversation Shiver asked for could not be opened (the core knows which it asked for) */
      openDmFailed: boolean;
      /** the page has something to show, so Shiver's loading view can go */
      ready: boolean;
      /** the client refused the session and is asking for credentials */
      signedOut: boolean;
      /** Sharkord's client never started in this page (the core revives it) */
      stalled: boolean;
      voice: VoiceSnapshot | null;
      /** the channel on screen, which counts as reading it */
      viewingChannelId: number | null;
      /** links to open in the browser */
      open: string[];
      /** something is fullscreen, so Shiver hides its overlay webviews */
      fullscreen: boolean;
    } | null;
    __SHIVER_SET_MUTED__?: (muted: number[]) => void;
    /** shows one conversation beside Shiver's DM list, or (null) goes back to the channels */
    __SHIVER_CONVERSATION__?: (name: string | null) => void;
    __SHIVER_SELECT_CHANNEL__?: (channelId: number) => void;
    __SHIVER_SET_READ_FLOOR__?: (floor: Record<string, number>) => void;
    __SHIVER_SET_THEME__?: (theme: ShiverTheme | null) => void;
    /** the window is minimised, which the page cannot otherwise tell */
    __SHIVER_SET_HIDDEN__?: (hidden: boolean) => void;
    __SHIVER_SET_VOICE_LOCK__?: (locked: boolean) => void;
    __SHIVER_VOICE__?: (action: VoiceAction) => void;
    __SHIVER_MARK_ALL_READ__?: () => void;
    /** the user changed which of this server's messages notify */
    __SHIVER_SET_NOTIFY__?: (level: NotifyLevel) => void;
  }
}
