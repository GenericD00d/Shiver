/**
 * The Shiver bridge (desktop): an initialization script in every Sharkord page, before the page's
 * own scripts. It is handed only this entry's config. Pages have no IPC, so it queues what it sees
 * and the core collects it through `__SHIVER_DRAIN__`; the core calls in through the other
 * `__SHIVER_*__` hooks. It reads Sharkord rather than driving it, except for opening a DM, voice
 * controls and the channel menu's items, which all use the page's own controls. A client that
 * speaks Sharkord's desktop api (`desktop-api.ts`) hands over its notifications and shortcuts
 * through that instead.
 */

import { channelPressed, installChannelMenu, requestChannelMenu } from '../../shared/web/bridge/channel-menu';
import { defineHook, ensureStyle, installExternalLinks, whenDocumentReady } from '../../shared/web/bridge/dom';
import {
  installAttachmentCards,
  installComposerWrap,
  installSidePanels,
  installSoundVolume,
  installVoiceColors
} from '../../shared/web/bridge/features';
import { watchDmActivity } from '../../shared/web/bridge/messages';
import { seedSharkordMutes, watchSharkordMutes } from '../../shared/web/bridge/mutes';
import { pushMutesToPlugin, storeReadFloor, syncMutesWithPlugin } from '../../shared/web/bridge/plugin';
import {
  CHANNEL_ITEM,
  COMPOSE_EDITOR,
  installMuteStyles,
  markAllChannelsRead,
  NARROW,
  notificationAuthor,
  notificationTarget,
  notifyAllows,
  type SharkordState,
  SIDE_PANEL,
  watchStore
} from '../../shared/web/bridge/sharkord';
import { applyPageTheme } from '../../shared/web/bridge/theme';
import { onOpenDmFailure, openDmChannelId, selectChannelWhenReady, showConversation } from './conversations';
import {
  connectionState,
  desktopMismatch,
  installDesktopApi,
  openNotification,
  pressHotkey,
  readHotkeyErrors
} from './desktop-api';
import {
  installNotificationWrapper,
  mentionsAny,
  ownMentionNames,
  resolveDmChannels,
  silenceMessagePing
} from './notifications';
import { createDisconnectWatch, watchDisconnectChoice } from '../../shared/web/bridge/disconnect';
import { createDmTracker } from './dm-tracker';
import { createHotkeyHandoff } from './hotkey-handoff';
import { capturePageErrors, takePageErrors } from './page-errors';
import { atConnectForm, isClientReady, isFullscreen, isSignedIn, isSignedOut, isStalled } from './page-state';
import { applyNotifyLevel, seedDefaults, seedSession } from './storage';
import type { QueuedMute, QueuedNotification, ShiverConfig } from './types';
import { setWindowHidden } from './visibility';
import { installVoiceLock, readVoice, runVoiceAction, setVoiceLocked } from './voice';

function install(shiver: ShiverConfig) {
  capturePageErrors();
  seedSession(shiver.token);
  seedDefaults(shiver.notify);

  // prototype patches, which must be in place before the page's scripts; the volume first, since
  // the ping filter captures the patched `connect`
  installSoundVolume(shiver.soundVolume);
  silenceMessagePing();

  let muted = new Set(shiver.muted);
  let state: SharkordState = {};
  let syncedMutes: number[] | null = null;
  let openDmFailed = false;
  let markAllRead = false;
  const dms = createDmTracker(shiver.origin);
  const hotkeys = createHotkeyHandoff();
  const disconnect = createDisconnectWatch();
  let notify = shiver.notify;
  /** set when the level changes on a loaded page, whose Sharkord read its switches at load */
  let notifyChangedLive = false;
  const queue: QueuedNotification[] = [];
  const openQueue: string[] = [];
  const muteQueue: QueuedMute[] = [];

  watchDmActivity({
    onMessage: dms.noteMessage,
    onDmTimes: (times) => {
      for (const [channelId, at] of times) dms.noteMessage(channelId, at);
    },
    onClosedChanged: dms.closedChanged
  });

  installExternalLinks((href) => openQueue.push(href));
  watchDisconnectChoice(disconnect.chosen);

  onOpenDmFailure(() => {
    openDmFailed = true;
  });

  // Sharkord's mute is the face of Shiver's: each toggle there goes to the core, which keeps the list
  // and hands it back through `__SHIVER_SET_MUTED__`
  watchSharkordMutes((channelId, isMuted) => muteQueue.push({ channelId, muted: isMuted }));
  seedSharkordMutes(muted);

  defineHook('__SHIVER_SET_MUTED__', (next) => {
    muted = new Set(next);
    seedSharkordMutes(muted);
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
    // the client's own word, when it gives one, over what the page shows
    const connection = connectionState();
    const drained = {
      notifications: resolveDmChannels(queue, state).splice(0),
      mutes: muteQueue.splice(0),
      markAllRead,
      dms: dms.take(),
      syncedMutes,
      open: openQueue.splice(0),
      openDmFailed,
      ready: isClientReady(state),
      signedOut: connection !== 'connected' && isSignedOut(),
      // ready at the connect form too, so signing in there needs saying
      signedIn: isSignedIn(state),
      userDisconnected: disconnect.take(connection !== 'connected' && atConnectForm()),
      stalled: connection !== 'connected' && connection !== 'connecting' && isStalled(),
      hotkeys: hotkeys.take(),
      mismatch: desktopMismatch(),
      pageErrors: takePageErrors(),
      fullscreen: isFullscreen(),
      voice: readVoice(state),
      // an open DM first: `selectedChannelId` keeps naming the last ordinary channel
      viewingChannelId: openDmChannelId(state) ?? (typeof state.selectedChannelId === 'number' ? state.selectedChannelId : null)
    };

    syncedMutes = null;
    openDmFailed = false;
    markAllRead = false;

    const { notifications, mutes, open, ready, signedIn, signedOut, stalled, fullscreen, voice, viewingChannelId, mismatch } =
      drained;
    const now = JSON.stringify([ready, signedIn, signedOut, stalled, fullscreen, voice, viewingChannelId, mismatch]);
    const queued =
      notifications.length ||
      mutes.length ||
      open.length ||
      drained.markAllRead ||
      drained.userDisconnected ||
      drained.dms ||
      drained.syncedMutes ||
      drained.openDmFailed ||
      drained.hotkeys ||
      drained.pageErrors.length;

    // a signed-out page keeps saying so: recovery retries on those reports
    if (!full && !queued && !signedOut && now === reported) return null;

    reported = now;

    return drained;
  });

  defineHook('__SHIVER_CONVERSATION__', showConversation);
  defineHook('__SHIVER_SELECT_CHANNEL__', (channelId, target) => {
    const open = () => {
      if (typeof channelId === 'number') selectChannelWhenReady(channelId);
    };
    const isDm = (id: number) => !!(state.channels ?? []).find((channel) => channel.id === id)?.isDm;

    // the message itself, through the client's own handling, when it offers one; else its channel
    if (!openNotification(typeof channelId === 'number' ? channelId : null, target ?? null, isDm, open)) open();
  });
  defineHook('__SHIVER_SET_READ_FLOOR__', (floor) => void storeReadFloor(floor));
  defineHook('__SHIVER_SET_THEME__', applyPageTheme);
  defineHook('__SHIVER_SET_HIDDEN__', setWindowHidden);

  /** whether a notification gets past Shiver's mutes and level */
  const allows = (channelId: number | null, isDm: boolean, body: string) => {
    if (channelId !== null && muted.has(channelId)) return false;

    // Sharkord applied the level it loaded with exactly; one changed since is guessed from the text
    const mentionsMe = !notifyChangedLive || mentionsAny(body, ownMentionNames(state));

    return notifyAllows(notify, isDm, mentionsMe);
  };

  installDesktopApi({
    notify: ({ title, body, icon, message, pluginRef }) => {
      if (!message) {
        // a plugin's own: from no channel, so only the level's floor applies (never in "DMs only")
        if (pluginRef === null || notify === 'dms') return;

        queue.push({ channelId: null, channelName: null, author: title, body, iconUrl: icon, isDm: false, target: { pluginRef } });

        return;
      }

      const { channelId, messageId, parentMessageId, isDm } = message;

      if (!allows(channelId, isDm, body)) return;

      const channel = (state.channels ?? []).find((candidate) => candidate.id === channelId);

      queue.push({
        channelId,
        channelName: isDm ? null : (channel?.name ?? null),
        author: notificationAuthor(title, channelId, state),
        body,
        iconUrl: icon,
        isDm,
        target: { messageId, threadId: parentMessageId ?? null }
      });
    },
    setHotkeys: hotkeys.set
  });

  defineHook('__SHIVER_HOTKEY__', pressHotkey);
  defineHook('__SHIVER_HOTKEYS_SET__', (errors) => hotkeys.answer(readHotkeyErrors(errors)));

  installNotificationWrapper((title, options) => {
    const { channelId, channelName, author, isDm } = notificationTarget(title, state);

    if (!allows(channelId, isDm, options?.body ?? '')) return;

    queue.push({ channelId, channelName, author, body: options?.body ?? '', iconUrl: options?.icon ?? null, isDm });
  });

  whenDocumentReady(() => {
    if (shiver.theme) applyPageTheme(shiver.theme);

    reserveTopBarSpace();
    installMuteStyles();
    installAttachmentCards(shiver.minimiseAttachments);
    installAttachmentFocus();
    installSpellcheck(shiver.spellcheck);
    installVoiceColors();
    installSidePanels();
    installComposerWrap();
    installVoiceLock(shiver.voiceLocked, () => state);

    if (shiver.conversation) showConversation(shiver.conversation);

    dms.start();
    watchStore((next) => {
      state = next;
      dms.update(next);
    });

    installChannelMenu({
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
      seedSharkordMutes(muted);
    });
  });
}

/**
 * Keeps Sharkord's controls clear of Shiver's bell, a 48px webview (`BELL_SIZE` in `webviews.rs`)
 * over the top-right corner: pads the top bar (found by its column template, its own) and a
 * side panel's header while it covers a narrow page, and moves a right-hand sheet's close button
 * out from under it.
 */
function reserveTopBarSpace() {
  ensureStyle('shiver-topbar-reserve').textContent = `
[class~="grid-cols-[1fr_minmax(320px,1.4fr)_1fr]"] { padding-right: 48px !important; }
@media ${NARROW} { ${SIDE_PANEL} .h-12.border-b { padding-right: 48px !important; } }
[data-slot="sheet-content"][class~="right-0"] > button[class~="top-4"][class~="right-4"] { right: 64px !important; }
`;
}

/**
 * Turns the page's spellchecking off or back on. Sharkord sets no `spellcheck` of its own, and the
 * attribute is inherited, so one on the root reaches every text box, the message box included.
 */
function installSpellcheck(on: boolean) {
  const apply = (next: unknown) => {
    if (next === false) {
      document.documentElement.spellcheck = false;
    } else {
      document.documentElement.removeAttribute('spellcheck');
    }
  };

  apply(on);
  defineHook('__SHIVER_SET_SPELLCHECK__', apply);
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
      cancelNativeFor = row && requestChannelMenu(row) ? event : null;
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
