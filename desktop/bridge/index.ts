/**
 * The Shiver bridge (desktop): an initialization script in every Sharkord page, before the page's
 * own scripts. It is handed only this entry's config. Pages have no IPC, so it queues what it sees
 * and the core collects it through `__SHIVER_DRAIN__`; the core calls in through the other
 * `__SHIVER_*__` hooks. It reads Sharkord rather than driving it, except for opening a DM, voice
 * controls and the channel menu's items, which all use the page's own controls.
 */

import { channelPressed, installChannelMenu, requestChannelMenu } from '../../shared/web/bridge/channel-menu';
import {
  defineHook,
  ensureStyle,
  installExternalLinks,
  onDomSettled,
  touched,
  whenDocumentReady
} from '../../shared/web/bridge/dom';
import {
  installAttachmentCards,
  installSidePanels,
  installSoundVolume,
  installVoiceColors
} from '../../shared/web/bridge/features';
import { watchDmActivity } from '../../shared/web/bridge/messages';
import { pushMutesToPlugin, storeReadFloor, syncMutesWithPlugin } from '../../shared/web/bridge/plugin';
import {
  CHANNEL_ITEM,
  COMPOSE_EDITOR,
  type DmChannel,
  installMuteStyles,
  markAllChannelsRead,
  NARROW,
  notificationTarget,
  notifyAllows,
  paintMuted,
  readDms,
  type SharkordState,
  SIDE_PANEL,
  watchStore
} from '../../shared/web/bridge/sharkord';
import { applyPageTheme } from '../../shared/web/bridge/theme';
import { onOpenDmFailure, openDmChannelId, selectChannelWhenReady, showConversation } from './conversations';
import {
  installNotificationWrapper,
  mentionsName,
  ownName,
  resolveDmChannels,
  silenceMessagePing
} from './notifications';
import { isClientReady, isFullscreen, isSignedOut, isStalled } from './page-state';
import { applyNotifyLevel, seedDefaults, seedSession } from './storage';
import type { QueuedMute, QueuedNotification, ShiverConfig } from './types';
import { setWindowHidden } from './visibility';
import { installVoiceLock, readVoice, runVoiceAction, setVoiceLocked } from './voice';

function install(shiver: ShiverConfig) {
  seedSession(shiver.token);
  seedDefaults(shiver.notify);

  // prototype patches, which must be in place before the page's scripts; the volume first, since
  // the ping filter captures the patched `connect`
  installSoundVolume(shiver.soundVolume);
  silenceMessagePing();

  let muted = new Set(shiver.muted);
  let state: SharkordState = {};
  let dms: DmChannel[] | null = null;
  let dmsSignature = '';
  let syncedMutes: number[] | null = null;
  let openDmFailed = false;
  let markAllRead = false;
  let notify = shiver.notify;
  /** set when the level changes on a loaded page, whose Sharkord read its switches at load */
  let notifyChangedLive = false;
  const queue: QueuedNotification[] = [];
  const openQueue: string[] = [];
  const muteQueue: QueuedMute[] = [];
  /** each channel's latest message, sent or received, as the page's connection reports it */
  const lastSeen = new Map<number, number>();
  let lastSeenVersion = 0;
  /** redraws the DM list; set once the page is ready */
  let refreshDms = () => {};
  /** a message in `channelId` at `at`, moving that conversation up if it is newer */
  const noteMessage = (channelId: number, at: number) => {
    if (at <= (lastSeen.get(channelId) ?? 0)) return false;

    lastSeen.set(channelId, at);
    lastSeenVersion += 1;
    refreshDms();

    return true;
  };

  watchDmActivity({
    onMessage: noteMessage,
    onDmTimes: (times) => {
      for (const [channelId, at] of times) noteMessage(channelId, at);
    }
  });

  installExternalLinks((href) => openQueue.push(href));

  onOpenDmFailure(() => {
    openDmFailed = true;
  });

  defineHook('__SHIVER_SET_MUTED__', (next) => {
    muted = new Set(next);
    paintMuted(muted);
    pushMutesToPlugin([...muted]);
  });

  defineHook('__SHIVER_SET_VOICE_LOCK__', setVoiceLocked);
  defineHook('__SHIVER_VOICE__', runVoiceAction);
  defineHook('__SHIVER_MARK_ALL_READ__', markAllChannelsRead);
  defineHook('__SHIVER_SET_NOTIFY__', (level) => {
    if (level !== 'all' && level !== 'mentions' && level !== 'dms') return;

    notify = level;
    notifyChangedLive = true;
    applyNotifyLevel(level, true);
  });

  let reported = '';

  defineHook('__SHIVER_DRAIN__', (full) => {
    const drained = {
      notifications: resolveDmChannels(queue, state).splice(0),
      mutes: muteQueue.splice(0),
      markAllRead,
      dms,
      syncedMutes,
      open: openQueue.splice(0),
      openDmFailed,
      ready: isClientReady(state),
      signedOut: isSignedOut(),
      stalled: isStalled(),
      fullscreen: isFullscreen(),
      voice: readVoice(state),
      // an open DM first: `selectedChannelId` keeps naming the last ordinary channel
      viewingChannelId: openDmChannelId(state) ?? (typeof state.selectedChannelId === 'number' ? state.selectedChannelId : null)
    };

    dms = null;
    syncedMutes = null;
    openDmFailed = false;
    markAllRead = false;

    const { notifications, mutes, open, ready, signedOut, stalled, fullscreen, voice, viewingChannelId } = drained;
    const now = JSON.stringify([ready, signedOut, stalled, fullscreen, voice, viewingChannelId]);
    const queued =
      notifications.length || mutes.length || open.length || drained.markAllRead || drained.dms || drained.syncedMutes || drained.openDmFailed;

    // a signed-out page keeps saying so: recovery retries on those reports
    if (!full && !queued && !signedOut && now === reported) return null;

    reported = now;

    return drained;
  });

  defineHook('__SHIVER_CONVERSATION__', showConversation);
  defineHook('__SHIVER_SELECT_CHANNEL__', selectChannelWhenReady);
  defineHook('__SHIVER_SET_READ_FLOOR__', (floor) => void storeReadFloor(floor));
  defineHook('__SHIVER_SET_THEME__', applyPageTheme);
  defineHook('__SHIVER_SET_HIDDEN__', setWindowHidden);

  installNotificationWrapper((title, options) => {
    const { channelId, channelName, author, isDm } = notificationTarget(title, state);

    if (channelId !== null && muted.has(channelId)) return;

    // Sharkord applied the level it loaded with exactly; one changed since is guessed from the text
    const mentionsMe = !notifyChangedLive || mentionsName(options?.body ?? '', ownName(state));

    if (!notifyAllows(notify, isDm, mentionsMe)) return;

    queue.push({ channelId, channelName, author, body: options?.body ?? '', iconUrl: options?.icon ?? null, isDm });
  });

  whenDocumentReady(() => {
    if (shiver.theme) applyPageTheme(shiver.theme);

    reserveTopBarSpace();
    installMuteStyles();
    installAttachmentCards(shiver.minimiseAttachments);
    installAttachmentFocus();
    installVoiceColors();
    installSidePanels();
    installVoiceLock(shiver.voiceLocked, () => state);

    if (shiver.conversation) showConversation(shiver.conversation);

    let dmInputs: unknown[] = [];

    // the DM list only changes with channels, users or a newly seen message
    refreshDms = () => {
      const next = state;
      const inputs = [next.channels, next.users, next.ownUserId, lastSeenVersion];

      if (inputs.every((input, index) => input === dmInputs[index])) return;

      dmInputs = inputs;

      const list = readDms(shiver.origin, next, lastSeen);

      // avatar urls carry expiring tokens, so they are left out of the comparison
      const signature = JSON.stringify(list.map(({ channelId, name, lastMessageAt }) => [channelId, name, lastMessageAt]));

      if (signature !== dmsSignature) {
        dmsSignature = signature;
        dms = list;
      }
    };

    watchStore((next) => {
      state = next;
      refreshDms();
    });

    installChannelMenu({
      mutes: {
        has: (channelId) => muted.has(channelId),
        // the core stores it and hands the page its new list
        toggle: (channelId) => muteQueue.push({ channelId, muted: !muted.has(channelId) })
      },
      markAllRead: () => {
        markAllChannelsRead();
        markAllRead = true;
      },
      touch: false
    });
    installChannelRightClick();

    void syncMutesWithPlugin([...muted]).then((merged) => {
      if (!merged) return;

      muted = new Set(merged);
      syncedMutes = merged;
      paintMuted(muted);
    });

    // rows Sharkord adds or redraws
    onDomSettled((changed) => paintMuted(muted, touched(changed, CHANNEL_ITEM)));
  });
}

/**
 * Keeps Sharkord's controls clear of Shiver's bell, a 48px webview (`BELL_SIZE` in `webviews.rs`)
 * over the top-right corner: pads the top bar (the only `h-12 w-full` element with `lg:grid`) and a
 * side panel's header while it covers a narrow page, and moves a right-hand sheet's close button
 * out from under it.
 */
function reserveTopBarSpace() {
  ensureStyle('shiver-topbar-reserve').textContent = `
.h-12.w-full[class~="lg:grid"] { padding-right: 48px !important; }
@media ${NARROW} { ${SIDE_PANEL} .h-12.border-b { padding-right: 48px !important; } }
[data-slot="sheet-content"][class~="right-0"] > button[class~="top-4"][class~="right-4"] { right: 64px !important; }
`;
}

/** Right-click on a sidebar channel opens the channel menu (`channel-menu.ts`), not the browser's. */
function installChannelRightClick() {
  let cancelNativeFor: Event | null = null;

  // capture: the press has to be noted before Radix renders its own menu during React's dispatch
  document.addEventListener(
    'contextmenu',
    (event) => {
      const row = (event.target as Element | null)?.closest?.(CHANNEL_ITEM) ?? null;

      channelPressed(row);
      cancelNativeFor = row && requestChannelMenu(row, event.clientX, event.clientY) ? event : null;
    },
    true
  );

  // Bubble phase: cancelling in capture would make Radix skip its own (already-prevented) handler.
  document.addEventListener('contextmenu', (event) => {
    if (event !== cancelNativeFor) return;

    cancelNativeFor = null;
    event.preventDefault();
  });
}

/**
 * Returns focus to the compose editor (caret at the end) once a file is picked or dropped, so Enter
 * sends it; picking a file leaves focus on the paperclip. Not for a file picked in a dialog (an
 * avatar), and never taking focus from another text field.
 */
function installAttachmentFocus() {
  const refocus = () => {
    const editor = document.querySelector<HTMLElement>(COMPOSE_EDITOR);
    const active = document.activeElement;

    if (!editor || document.querySelector('[role="dialog"]')) return;
    if (active instanceof HTMLElement && active !== editor && (active.isContentEditable || active.matches('input:not([type="file"]), textarea, select'))) return;

    editor.focus();

    const range = document.createRange();
    const selection = window.getSelection();

    range.selectNodeContents(editor);
    range.collapse(false);
    selection?.removeAllRanges();
    selection?.addRange(range);
  };

  // after the page has taken the file
  document.addEventListener('change', (event) => {
    if (event.target instanceof HTMLInputElement && event.target.type === 'file') window.setTimeout(refocus);
  }, true);
  document.addEventListener('drop', (event) => {
    if (event.dataTransfer?.files.length) window.setTimeout(refocus);
  }, true);
}

// Read and removed from the page before anything else runs (`bridge_script` in webviews.rs runs
// this only in the top frame on the entry's origin).
const config = window.__SHIVER__;

delete window.__SHIVER__;

if (config) {
  try {
    install(config);
  } catch (error) {
    console.error('[shiver] bridge failed to install', error);
  }
}
