/**
 * The Shiver bridge (desktop): an initialization script in every Sharkord page, before the page's
 * own scripts. It is handed only this entry's config. Pages have no IPC, so it queues what it sees
 * and the core collects it through `__SHIVER_DRAIN__`; the core calls in through the other
 * `__SHIVER_*__` hooks. It reads Sharkord rather than driving it, except for opening a DM, voice
 * controls and the channel menu's mute item, which all use the page's own controls.
 */

import { defineHook, ensureStyle, installExternalLinks, isTopFrame, onDomSettled, openMenuOnScreen, addedMenu, whenDocumentReady } from '../../shared/web/bridge/dom';
import { installAttachmentCards, installRoleColors, installSoundVolume, installStatusButton, installVoiceColors } from '../../shared/web/bridge/features';
import { pushMutesToPlugin, storeReadFloor, syncMutesWithPlugin } from '../../shared/web/bridge/plugin';
import {
  addMuteItem,
  CHANNEL_ITEM,
  channelOfRow,
  CONNECT_FORM,
  DM_ITEM,
  DM_TOGGLE,
  installMuteStyles,
  markAllChannelsRead,
  paintMuted,
  rowName,
  SERVER_VIEW,
  SIDEBAR,
  sharkordStore,
  type SharkordChannel,
  type SharkordFile,
  type SharkordState,
  watchStore
} from '../../shared/web/bridge/sharkord';
import { applyPageTheme, type ShiverTheme } from '../../shared/web/bridge/theme';
import { AUTO_LOGIN, AUTO_LOGIN_TOKEN, installSessionShim } from '../../shared/web/session';

type ShiverConfig = {
  entryId: string;
  origin: string;
  /** null while the user is on Sharkord's own colours */
  theme: ShiverTheme | null;
  /** the session Shiver signed in with; served to the page from memory, never stored */
  token: string | null;
  muted: number[];
  /** `server` browses channels and reports; `dm` shows one conversation and reports nothing else */
  role: 'server' | 'dm';
  /** for a `dm` page, the conversation to open once possible */
  openDm: string | null;
  /** another server holds the voice session (which one is never said), so joins are refused */
  voiceLocked: boolean;
  minimiseAttachments: boolean;
  /** percentage of Sharkord's own sound level */
  soundVolume: number;
};

type QueuedNotification = {
  channelId: number | null;
  channelName: string | null;
  author: string;
  body: string;
  iconUrl: string | null;
  isDm: boolean;
};

type QueuedMute = { channelId: number; muted: boolean };

type DmChannel = { channelId: number; name: string; iconUrl: string | null; lastMessageAt: number | null };

/** The voice session on this server: channel from the store, mute flags read off Sharkord's controls. */
type VoiceSnapshot = {
  channelId: number;
  channelName: string | null;
  micMuted: boolean;
  soundMuted: boolean;
  /** Sharkord disables its mic button while deafened */
  micLocked: boolean;
};

type VoiceAction = 'mic' | 'sound' | 'leave';

declare global {
  interface Window {
    __SHIVER__?: ShiverConfig;
    /** everything queued since the last call; the core drains it on a timer */
    __SHIVER_DRAIN__?: () => {
      notifications: QueuedNotification[];
      dms: DmChannel[] | null;
      mutes: QueuedMute[];
      /** the plugin's reconciled mute list, sent once */
      syncedMutes: number[] | null;
      /** a conversation Shiver asked for and could not open */
      openDmFailed: string | null;
      /** the page has something to show, so Shiver's loading view can go */
      ready: boolean;
      /** the client refused the session and is asking for credentials */
      signedOut: boolean;
      voice: VoiceSnapshot | null;
      /** the channel on screen, which counts as reading it */
      viewingChannelId: number | null;
      /** links to open in the browser */
      open: string[];
      /** something is fullscreen, so Shiver hides its overlay webviews */
      fullscreen: boolean;
    };
    __SHIVER_SET_MUTED__?: (muted: number[]) => void;
    __SHIVER_OPEN_DM__?: (name: string) => void;
    __SHIVER_SELECT_CHANNEL__?: (channelId: number) => void;
    __SHIVER_SET_READ_FLOOR__?: (floor: Record<string, number>) => void;
    __SHIVER_SET_THEME__?: (theme: ShiverTheme | null) => void;
    /** the window is minimised, which the page cannot otherwise tell */
    __SHIVER_SET_HIDDEN__?: (hidden: boolean) => void;
    __SHIVER_SET_VOICE_LOCK__?: (locked: boolean) => void;
    __SHIVER_VOICE__?: (action: VoiceAction) => void;
    __SHIVER_MARK_ALL_READ__?: () => void;
  }
}

const OPEN_DM_TIMEOUT_MS = 25_000;

// taken before the page's scripts run, so a page cannot fake fullscreen to hide Shiver's chrome
const nativeFullscreenElement = Object.getOwnPropertyDescriptor(Document.prototype, 'fullscreenElement')?.get;
const nativeApply = Reflect.apply;
const isFullscreen = () => !!nativeFullscreenElement && nativeApply(nativeFullscreenElement, document, []) !== null;

function install(shiver: ShiverConfig) {
  seedSession(shiver.token);

  if (shiver.role === 'dm') {
    installConversationView(shiver);

    return;
  }

  seedDefaults();

  // prototype patches, which must be in place before the page's scripts; the volume first, since
  // the ping filter captures the patched `connect`
  installSoundVolume(shiver.soundVolume);
  silenceMessagePing();

  let muted = new Set(shiver.muted);
  let state: SharkordState = {};
  let dms: DmChannel[] | null = null;
  let dmsSignature = '';
  let syncedMutes: number[] | null = null;
  let openDmFailure: string | null = null;
  const queue: QueuedNotification[] = [];
  const openQueue: string[] = [];
  const muteQueue: QueuedMute[] = [];
  /** when a message was last seen per channel (notifications are all the bridge sees of them) */
  const lastSeen = new Map<number, number>();
  let lastSeenVersion = 0;

  installExternalLinks((href) => openQueue.push(href));

  reportOpenDmFailure = (name) => {
    openDmFailure = name;
  };

  defineHook('__SHIVER_SET_MUTED__', (next) => {
    muted = new Set(next);
    paintMuted(muted);
    pushMutesToPlugin([...muted]);
  });

  defineHook('__SHIVER_SET_VOICE_LOCK__', (locked) => {
    voiceLocked = locked;
  });
  defineHook('__SHIVER_VOICE__', runVoiceAction);
  defineHook('__SHIVER_MARK_ALL_READ__', markAllChannelsRead);

  defineHook('__SHIVER_DRAIN__', () => {
    const drained = {
      notifications: resolveDmChannels(queue, state).splice(0),
      mutes: muteQueue.splice(0),
      dms,
      syncedMutes,
      open: openQueue.splice(0),
      openDmFailed: openDmFailure,
      ready: isClientReady(state),
      signedOut: isSignedOut(),
      fullscreen: isFullscreen(),
      voice: readVoice(state),
      // an open DM first: `selectedChannelId` keeps naming the last ordinary channel
      viewingChannelId: openDmChannelId(state) ?? (typeof state.selectedChannelId === 'number' ? state.selectedChannelId : null)
    };

    dms = null;
    syncedMutes = null;
    openDmFailure = null;

    return drained;
  });

  defineHook('__SHIVER_OPEN_DM__', openDirectMessage);
  defineHook('__SHIVER_SELECT_CHANNEL__', selectChannelWhenReady);
  defineHook('__SHIVER_SET_READ_FLOOR__', (floor) => void storeReadFloor(floor));
  defineHook('__SHIVER_SET_THEME__', applyPageTheme);
  defineHook('__SHIVER_SET_HIDDEN__', setWindowHidden);

  installNotificationWrapper((title, options) => {
    const channelName = parseChannelName(title);
    const author = parseAuthor(title);
    const isDm = title.includes('(DM)');
    // matched by name: `selectedChannelId` is exactly the channel a notification is not for
    const channelId = isDm
      ? findDmChannelIdByUserName(state, author)
      : ((state.channels ?? []).find((channel) => !channel.isDm && channel.name === channelName)?.id ?? null);

    if (channelId !== null) {
      lastSeen.set(channelId, Date.now());
      lastSeenVersion += 1;

      if (muted.has(channelId)) return;
    }

    queue.push({ channelId, channelName, author, body: options?.body ?? '', iconUrl: options?.icon ?? null, isDm });
  });

  whenDocumentReady(() => {
    if (shiver.theme) applyPageTheme(shiver.theme);

    reserveTopBarSpace();
    installMuteStyles();
    installAttachmentCards(shiver.minimiseAttachments);
    installAttachmentFocus();
    installVoiceColors();
    installVoiceLock(shiver.voiceLocked, () => state);

    let dmInputs: unknown[] = [];

    watchStore((next) => {
      state = next;
      paintMuted(muted);

      // the DM list only changes with channels, users or a newly seen message
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
    });

    installRoleColors();
    watchChannelContextMenu(() => muted, muteQueue);
    installStatusButton(false);

    void syncMutesWithPlugin([...muted]).then((merged) => {
      if (!merged) return;

      muted = new Set(merged);
      syncedMutes = merged;
      paintMuted(muted);
    });

    // Sharkord re-renders the channel list constantly, dropping the dimming
    onDomSettled(() => paintMuted(muted));
  });
}

/**
 * A second page for the same server showing one DM. It hides its sidebar, throws its notifications
 * away (the server page reports them) and reports only a failed open, links and the channel on
 * screen.
 */
function installConversationView(shiver: ShiverConfig) {
  const openQueue: string[] = [];
  let openDmFailure: string | null = null;

  installExternalLinks((href) => openQueue.push(href));
  installSoundVolume(shiver.soundVolume);
  silenceMessagePing();
  installNotificationWrapper(() => undefined);

  reportOpenDmFailure = (name) => {
    openDmFailure = name;
  };

  defineHook('__SHIVER_OPEN_DM__', openDirectMessage);
  defineHook('__SHIVER_SET_THEME__', applyPageTheme);

  defineHook('__SHIVER_DRAIN__', () => {
    const failure = openDmFailure;
    const selected = sharkordStore()?.getState().selectedChannelId;

    openDmFailure = null;

    return {
      notifications: [],
      mutes: [],
      dms: null,
      syncedMutes: null,
      open: openQueue.splice(0),
      openDmFailed: failure,
      ready: false,
      signedOut: false,
      fullscreen: false,
      voice: null,
      viewingChannelId: typeof selected === 'number' ? selected : null
    };
  });

  whenDocumentReady(() => {
    if (shiver.theme) applyPageTheme(shiver.theme);

    reserveTopBarSpace();
    hideSidebar();
    installRoleColors();
    installAttachmentCards(shiver.minimiseAttachments);
    installAttachmentFocus();
    installVoiceColors();

    if (shiver.openDm) openDirectMessage(shiver.openDm);

    // DM mode has to survive the sidebar being re-rendered as the client connects
    onDomSettled(hideSidebar);
  });
}

/**
 * Serves Shiver's session to Sharkord's auto-login from memory (see `shared/web/session.ts`).
 * Without a session, any copy an older Shiver left in storage is removed so a log out sticks.
 */
function seedSession(token: string | null) {
  if (token) {
    installSessionShim(token);

    return;
  }

  try {
    localStorage.removeItem(AUTO_LOGIN);
    localStorage.removeItem(AUTO_LOGIN_TOKEN);
  } catch {
    // storage blocked: the page shows its own login
  }
}

/**
 * Sharkord settings Shiver depends on, written only when absent so the user's own choices stand:
 * browser notifications on (Shiver's feed is built from them; mention-only off, since Shiver
 * filters with its own mutes) and rejoining the last channel on connect.
 */
function seedDefaults() {
  const defaults: Record<string, string> = {
    'sharkord-browser-notifications': 'true',
    'sharkord-browser-notifications-for-dms': 'true',
    'sharkord-browser-notifications-for-replies': 'true',
    'sharkord-browser-notifications-for-mentions': 'false',
    'sharkord-auto-join-last-channel': 'true'
  };

  try {
    for (const [key, value] of Object.entries(defaults)) {
      if (localStorage.getItem(key) === null) localStorage.setItem(key, value);
    }
  } catch {
    // storage blocked
  }
}

/** Sharkord's incoming-message tone: a single 600 Hz sine (`sfxMessageReceived`). */
const MESSAGE_PING_HZ = 600;

/**
 * Swallows Sharkord's incoming-message ping, which it plays for every message regardless of
 * Shiver's mutes; Shiver plays its own for the notifications that survive them. Only that tone is
 * dropped (matched on the frequency Sharkord requested): every other sound still plays.
 */
function silenceMessagePing() {
  const AudioContextCtor =
    window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;

  if (!AudioContextCtor?.prototype) return;

  const createOscillator = AudioContextCtor.prototype.createOscillator;
  const connect = AudioNode.prototype.connect as (this: AudioNode, destination: AudioNode) => AudioNode;

  AudioContextCtor.prototype.createOscillator = function patched(this: AudioContext) {
    const oscillator = createOscillator.call(this);
    const frequency = oscillator.frequency;
    const setValueAtTime = frequency.setValueAtTime.bind(frequency);
    let requestedHz: number | null = null;

    frequency.setValueAtTime = (value: number, when: number) => {
      requestedHz = value;

      return setValueAtTime(value, when);
    };

    // returning the destination keeps chained `.connect()` calls working
    oscillator.connect = function patchedConnect(this: OscillatorNode, destination: AudioNode) {
      return this.type === 'sine' && requestedHz === MESSAGE_PING_HZ ? destination : connect.call(this, destination);
    } as OscillatorNode['connect'];

    return oscillator;
  };
}

/**
 * Keeps Sharkord's controls clear of Shiver's bell, a 48px webview (`BELL_SIZE` in `webviews.rs`)
 * over the top-right corner: pads the top bar (the only `h-12 w-full` element with `lg:grid`) and
 * moves a right-hand sheet's close button out from under it.
 */
function reserveTopBarSpace() {
  ensureStyle('shiver-topbar-reserve').textContent = `
.h-12.w-full[class~="lg:grid"] { padding-right: 48px !important; }
[data-slot="sheet-content"][class~="right-0"] > button[class~="top-4"][class~="right-4"] { right: 64px !important; }
`;
}

/**
 * Replaces `window.Notification` so the page's notifications go to `handle` instead of the OS.
 * Claims permission, since Sharkord composes nothing without it.
 */
function installNotificationWrapper(handle: (title: string, options?: NotificationOptions) => void) {
  const Wrapped = function (title: string, options?: NotificationOptions) {
    handle(title, options);

    return {
      close: () => undefined,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      dispatchEvent: () => false
    } as unknown as Notification;
  } as unknown as typeof Notification;

  Object.defineProperty(Wrapped, 'permission', { get: () => 'granted' });
  Wrapped.requestPermission = () => Promise.resolve('granted' as NotificationPermission);

  try {
    Object.defineProperty(window, 'Notification', { configurable: true, writable: true, value: Wrapped });
  } catch {
    // cannot be replaced: the page keeps its own
  }
}

/** Sharkord titles read `Author in #channel` or `Author (DM)`. */
const parseAuthor = (title: string) => (title.split(/ in #| \(DM\)/)[0] ?? title).trim();
const parseChannelName = (title: string) => title.match(/ in #(.+)$/)?.[1] ?? null;

/** The other participant of a DM channel, named `DM - <userA>:<userB>`. */
function dmPartnerId(channel: SharkordChannel, ownUserId: number | undefined) {
  const match = channel.name.match(/^DM - (\d+):(\d+)$/);

  if (!match) return null;

  const [a, b] = [Number(match[1]), Number(match[2])];

  return ownUserId !== undefined && a === ownUserId ? b : a;
}

function findDmChannelIdByUserName(state: SharkordState, name: string) {
  const user = (state.users ?? []).find((candidate) => candidate.name === name);

  if (!user) return null;

  return (state.channels ?? []).find((channel) => channel.isDm && dmPartnerId(channel, state.ownUserId) === user.id)?.id ?? null;
}

/**
 * Retries the channel of queued DM notifications captured before the store had users and DM
 * channels; a notification without a channel could never be cleared by reading.
 */
function resolveDmChannels(queue: QueuedNotification[], state: SharkordState) {
  for (const queued of queue) {
    if (queued.isDm && queued.channelId === null) queued.channelId = findDmChannelIdByUserName(state, queued.author);
  }

  return queue;
}

/** The name a DM row shows: its name span (not the avatar's initials fallback span). */
const dmRowName = (row: HTMLElement) => row.querySelector<HTMLElement>('span.flex-1, span.truncate')?.textContent?.trim() ?? null;

/**
 * The DM open in Sharkord's DM view. Not in the plugin store (`selectedDmChannelId` lives in
 * another slice), so it is read off the highlighted row, by the same name notifications resolve by.
 */
function openDmChannelId(state: SharkordState) {
  const row = [...document.querySelectorAll<HTMLElement>(DM_ITEM)].find((candidate) => candidate.classList.contains('bg-accent'));

  if (!row) return null;

  let name = dmRowName(row);

  if (name === null) {
    // markup changed: the longest user name in the row, so "Ana" never beats "Ana B"
    const text = row.textContent ?? '';

    for (const user of state.users ?? []) {
      if (user.name && text.includes(user.name) && user.name.length > (name?.length ?? 0)) name = user.name;
    }
  }

  return name ? findDmChannelIdByUserName(state, name) : null;
}

function fileUrl(origin: string, file: SharkordFile | null | undefined) {
  if (!file) return null;

  try {
    const url = new URL(`/public/${file.name.split('/').map(encodeURIComponent).join('/')}`, origin);

    if (file._accessToken) {
      url.searchParams.set('accessToken', file._accessToken);

      if (file._accessTokenExpiresAt) url.searchParams.set('expires', String(file._accessTokenExpiresAt));
    }

    return url.toString();
  } catch {
    return null;
  }
}

/**
 * This server's DMs, newest first. A DM whose partner is not in the store yet is skipped rather
 * than listed under its raw channel name.
 */
function readDms(origin: string, state: SharkordState, lastSeen: Map<number, number>): DmChannel[] {
  const users = new Map((state.users ?? []).map((user) => [user.id, user]));
  const list: DmChannel[] = [];

  for (const channel of state.channels ?? []) {
    if (!channel.isDm) continue;

    const partner = users.get(dmPartnerId(channel, state.ownUserId) ?? -1);

    if (partner) {
      list.push({ channelId: channel.id, name: partner.name, iconUrl: fileUrl(origin, partner.avatar), lastMessageAt: lastSeen.get(channel.id) ?? null });
    }
  }

  // a stable order, so the inbox does not reorder under the pointer
  return list.sort((a, b) => (b.lastMessageAt ?? 0) - (a.lastMessageAt ?? 0) || a.name.localeCompare(b.name));
}

/** Hides Sharkord's sidebar, since Shiver's DM list is beside the page. */
function hideSidebar() {
  ensureStyle('shiver-dm-mode').textContent = `${SIDEBAR} { display: none !important; }`;
}

let openDmTimer: number | null = null;
/** set by the installers, so a failed open reaches the next drain */
let reportOpenDmFailure: (name: string) => void = () => undefined;

/**
 * Opens the DM with `name` by clicking Sharkord's own controls: the DM toggle (pressed at most
 * once, only while no DM rows exist, since it is a toggle) and then the row. Retries until a
 * deadline long enough to outlast a cold page connecting; a newer request replaces an older one.
 */
function openDirectMessage(name: string) {
  if (openDmTimer !== null) window.clearInterval(openDmTimer);

  openDmTimer = null;

  let toggled = false;
  const deadline = Date.now() + OPEN_DM_TIMEOUT_MS;

  const attempt = () => {
    const rows = [...document.querySelectorAll<HTMLElement>(DM_ITEM)];

    if (rows.length === 0) {
      const toggle = toggled ? null : document.querySelector<HTMLElement>(DM_TOGGLE);

      toggle?.click();
      toggled ||= !!toggle;
    } else {
      const row = rows.find((candidate) => {
        const named = dmRowName(candidate);

        // without the name span, any span that is exactly the name (never a substring)
        return named !== null ? named === name : [...candidate.querySelectorAll('span')].some((span) => span.textContent?.trim() === name);
      });

      if (row) {
        row.click();

        return true;
      }
    }

    if (Date.now() <= deadline) return false;

    reportOpenDmFailure(name);

    return true;
  };

  if (attempt()) return;

  openDmTimer = window.setInterval(() => {
    if (!attempt()) return;

    if (openDmTimer !== null) window.clearInterval(openDmTimer);

    openDmTimer = null;
  }, 150);
}

/**
 * Whether the client gave up on the session: the connect form is up and auto-login is no longer
 * `true` (Sharkord sets it false when it refuses a token; absent means nothing was seeded).
 */
function isSignedOut() {
  if (!document.querySelector(CONNECT_FORM)) return false;

  try {
    return localStorage.getItem(AUTO_LOGIN) !== 'true';
  } catch {
    return true;
  }
}

/** Connected (own user known and the server view mounted), or showing its connect form. */
function isClientReady(state: SharkordState) {
  if (document.querySelector(CONNECT_FORM)) return true;

  return typeof state.ownUserId === 'number' && document.querySelector(SERVER_VIEW) !== null;
}

/** Whether `element` contains lucide icon `name` (whole class token: `mic` is a prefix of `mic-off`). */
function hasLucideIcon(element: Element, name: string) {
  return [...element.querySelectorAll('svg')].some(
    (icon) => icon.classList.contains(`lucide-${name}`) || icon.classList.contains(`lucide-${name}-icon`)
  );
}

/**
 * Sharkord's own voice controls. Mic and deafen are the pair of buttons sharing a parent (the same
 * icons appear per participant in a voice channel's user list, but not as sibling buttons).
 */
function findVoiceButtons(): Record<VoiceAction, HTMLButtonElement | null> {
  const scope = document.querySelector(SIDEBAR) ?? document.body;
  const buttons = [...scope.querySelectorAll<HTMLButtonElement>('button')];
  const isMic = (button: Element) => hasLucideIcon(button, 'mic') || hasLucideIcon(button, 'mic-off');
  const isSound = (button: Element) => hasLucideIcon(button, 'headphones') || hasLucideIcon(button, 'headphone-off');
  const soundBeside = (button: Element) =>
    ([...(button.parentElement?.children ?? [])].find((sibling) => sibling !== button && isSound(sibling)) as HTMLButtonElement | undefined) ?? null;

  const mic = buttons.find((button) => isMic(button) && soundBeside(button)) ?? null;

  return {
    mic,
    sound: mic ? soundBeside(mic) : null,
    leave: buttons.find((button) => hasLucideIcon(button, 'phone-off')) ?? null
  };
}

function readVoice(state: SharkordState): VoiceSnapshot | null {
  const channelId = state.currentVoiceChannelId;

  if (typeof channelId !== 'number') return null;

  const { mic, sound } = findVoiceButtons();

  // missing controls read as unmuted: claiming a live mic is muted would be worse
  return {
    channelId,
    channelName: (state.channels ?? []).find((channel) => channel.id === channelId)?.name ?? null,
    micMuted: mic ? hasLucideIcon(mic, 'mic-off') : false,
    soundMuted: sound ? hasLucideIcon(sound, 'headphone-off') : false,
    micLocked: mic?.disabled ?? false
  };
}

/** Clicks one of Sharkord's voice controls; unknown actions and disabled controls do nothing. */
function runVoiceAction(action: VoiceAction) {
  const buttons = findVoiceButtons();
  const button = Object.hasOwn(buttons, action) ? buttons[action] : null;

  if (button && !button.disabled) button.click();
}

/** Selects a channel (for a clicked notification) once the client can, until a deadline. */
function selectChannelWhenReady(channelId: number) {
  const deadline = Date.now() + OPEN_DM_TIMEOUT_MS;

  const attempt = () => {
    const store = sharkordStore();
    const selectChannel = store?.actions?.selectChannel;

    if (selectChannel && (store.getState().channels ?? []).some((channel) => channel.id === channelId)) {
      selectChannel(channelId);

      return true;
    }

    return Date.now() > deadline;
  };

  if (attempt()) return;

  const timer = window.setInterval(() => {
    if (attempt()) window.clearInterval(timer);
  }, 250);
}

let voiceLocked = false;
let voiceNoticeTimer: number | null = null;

/**
 * Refuses a click on a voice channel while another server holds the call (capture phase on the
 * document, before React sees it), so the second join never happens. Rows carry only a name, so
 * voice rows are recognised through the store.
 */
function installVoiceLock(initial: boolean, getState: () => SharkordState) {
  voiceLocked = initial;

  document.addEventListener(
    'click',
    (event) => {
      if (!voiceLocked) return;

      const row = (event.target as Element | null)?.closest?.(CHANNEL_ITEM);

      if (!row) return;

      const name = rowName(row);

      if (!(getState().channels ?? []).some((channel) => channel.type === 'VOICE' && channel.name === name)) return;

      event.preventDefault();
      event.stopImmediatePropagation();
      showVoiceNotice();
    },
    true
  );
}

/** Says, where the user clicked, why the voice channel did not open (without naming the other server). */
function showVoiceNotice() {
  const id = 'shiver-voice-notice';
  const notice = document.getElementById(id) ?? document.body.appendChild(document.createElement('div'));

  notice.id = id;
  notice.textContent = 'You are already in a voice channel on another server in Shiver.';
  notice.setAttribute('role', 'status');
  notice.setAttribute(
    'style',
    'position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%); z-index: 2147483647; ' +
      'padding: 10px 16px; border-radius: 8px; font-size: 13px; pointer-events: none; ' +
      'background: var(--popover, #171717); color: var(--popover-foreground, #fafafa); ' +
      'border: 1px solid var(--border, rgba(250,250,250,0.12)); box-shadow: 0 8px 24px rgba(0,0,0,0.35)'
  );

  if (voiceNoticeTimer !== null) window.clearTimeout(voiceNoticeTimer);

  voiceNoticeTimer = window.setTimeout(() => {
    notice.remove();
    voiceNoticeTimer = null;
  }, 4000);
}

/** How long Shiver waits for Sharkord's own channel menu before drawing its own. */
const SHARKORD_MENU_GRACE_MS = 250;
/** How long after a right-click a newly added menu is taken to belong to it. */
const SHARKORD_MENU_WINDOW_MS = 2000;
const OWN_MENU_ID = 'shiver-channel-menu';

/**
 * Right-click on a channel: adds "Mute in Shiver" to Sharkord's menu, or, for users Sharkord gives
 * no menu (it has one only for channel managers), draws Shiver's own one-item menu after a short
 * grace. A late Sharkord menu still gets the item and replaces Shiver's.
 */
function watchChannelContextMenu(getMuted: () => Set<number>, muteQueue: QueuedMute[]) {
  let pending: SharkordChannel | null = null;
  let fallback = 0;
  /** what the last right-click was on, kept past `settle` for late menus */
  let lastChannel: SharkordChannel | null = null;
  let lastAt = 0;
  let cancelNativeFor: Event | null = null;

  const toggleFor = (channel: SharkordChannel) => () => muteQueue.push({ channelId: channel.id, muted: !getMuted().has(channel.id) });

  const settle = () => {
    window.clearTimeout(fallback);
    fallback = 0;
    pending = null;
  };

  // capture: the bookkeeping has to happen before Radix renders its menu during React's dispatch
  document.addEventListener(
    'contextmenu',
    (event) => {
      settle();
      closeOwnMenu();
      cancelNativeFor = null;

      const row = (event.target as Element | null)?.closest?.(CHANNEL_ITEM);

      pending = row ? channelOfRow(row) : null;

      if (!pending) return;

      lastChannel = pending;
      lastAt = Date.now();
      cancelNativeFor = event;

      const { clientX, clientY } = event;

      fallback = window.setTimeout(() => {
        const channel = pending;

        settle();

        if (!channel) return;

        // Radix moves an already-open menu rather than adding one, so the observer never sees it
        const open = openMenuOnScreen();

        if (open) addMuteItem(open, getMuted().has(channel.id), toggleFor(channel));
        else openOwnMenu(clientX, clientY, getMuted().has(channel.id), toggleFor(channel));
      }, SHARKORD_MENU_GRACE_MS);
    },
    true
  );

  // Bubble phase: cancelling in capture would make Radix skip its own (already-prevented) handler.
  document.addEventListener('contextmenu', (event) => {
    if (event !== cancelNativeFor) return;

    cancelNativeFor = null;
    event.preventDefault();
  });

  new MutationObserver((records) => {
    if (!lastChannel || Date.now() - lastAt >= SHARKORD_MENU_WINDOW_MS) return;

    const menu = addedMenu(records);

    if (!menu) return;

    addMuteItem(menu, getMuted().has(lastChannel.id), toggleFor(lastChannel));
    settle();
    closeOwnMenu();
  }).observe(document.body, { childList: true, subtree: true });
}

const closeOwnMenu = () => document.getElementById(OWN_MENU_ID)?.remove();

/**
 * Shiver's one-item channel menu, in a closed shadow root. It closes on a press outside it (not
 * any press, or the item's own click would never land), a scroll or Escape.
 */
function openOwnMenu(x: number, y: number, isMuted: boolean, toggle: () => void) {
  closeOwnMenu();

  const host = document.createElement('div');
  const root = host.attachShadow({ mode: 'closed' });
  const menu = document.createElement('div');
  const item = document.createElement('button');

  host.id = OWN_MENU_ID;
  root.innerHTML = `<style>
:host { position: fixed; inset: 0; z-index: 2147483646; }
.menu { position: absolute; min-width: 180px; padding: 4px; border-radius: 8px;
  border: 1px solid rgb(255 255 255 / 12%); background: #1f1f1f; color: #fafafa;
  box-shadow: 0 12px 32px rgb(0 0 0 / 55%); font: 500 13px/1.2 system-ui, -apple-system, "Segoe UI", sans-serif; }
.item { display: block; width: 100%; padding: 8px 10px; border: none; border-radius: 6px;
  background: none; color: inherit; font: inherit; text-align: left; cursor: default; }
.item:hover { background: #333333; }
</style>`;

  item.className = 'item';
  item.type = 'button';
  item.textContent = isMuted ? 'Unmute in Shiver' : 'Mute in Shiver';
  menu.className = 'menu';
  menu.style.left = `${Math.min(x, window.innerWidth - 200)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - 60)}px`;
  menu.append(item);
  root.append(menu);
  document.body.append(host);

  const dismiss = () => {
    closeOwnMenu();
    document.removeEventListener('mousedown', onMouseDown, true);
    document.removeEventListener('scroll', dismiss, true);
    document.removeEventListener('keydown', onKey, true);
  };

  // events from the closed shadow root are retargeted to the host
  const onMouseDown = (event: MouseEvent) => {
    if (!(event.target instanceof Node && host.contains(event.target))) dismiss();
  };

  const onKey = (event: KeyboardEvent) => {
    if (event.key === 'Escape') dismiss();
  };

  item.addEventListener('click', () => {
    toggle();
    dismiss();
  });

  document.addEventListener('mousedown', onMouseDown, true);
  document.addEventListener('scroll', dismiss, true);
  document.addEventListener('keydown', onKey, true);
}

let windowHidden = false;
let visibilityPatched = false;

/**
 * Makes `document.hidden` true while Shiver is minimised (a child webview stays "visible"), so
 * Sharkord notifies for messages in the channel on screen. Falls back to the real value otherwise.
 */
function setWindowHidden(hidden: boolean) {
  if (!visibilityPatched) {
    const native = (name: 'hidden' | 'visibilityState') => Object.getOwnPropertyDescriptor(Document.prototype, name)?.get;
    const nativeHidden = native('hidden');
    const nativeState = native('visibilityState');

    try {
      Object.defineProperty(document, 'hidden', {
        configurable: true,
        get: () => windowHidden || (nativeHidden?.call(document) ?? false)
      });
      Object.defineProperty(document, 'visibilityState', {
        configurable: true,
        get: () => (windowHidden ? 'hidden' : (nativeState?.call(document) ?? 'visible'))
      });
      visibilityPatched = true;
    } catch {
      // sealed by the page
    }
  }

  if (windowHidden === hidden) return;

  windowHidden = hidden;
  document.dispatchEvent(new Event('visibilitychange'));
}

/** Sharkord's compose editor, and a file waiting to be sent. */
const COMPOSE_EDITOR = '[data-testid="message-compose-editor"]';
const PENDING_FILE = 'div[class~="w-48"][class~="group"][class~="rounded-lg"]';

/**
 * Returns focus to the compose editor (caret at the end) when a file is attached, so Enter sends
 * it; picking a file leaves focus on the paperclip. Never steals focus from another text field.
 */
function installAttachmentFocus() {
  let pending = 0;

  onDomSettled(() => {
    const now = document.querySelectorAll(PENDING_FILE).length;
    const gained = now > pending;

    pending = now;

    if (!gained) return;

    const editor = document.querySelector<HTMLElement>(COMPOSE_EDITOR);
    const active = document.activeElement;

    if (!editor) return;
    if (active instanceof HTMLElement && active !== editor && (active.isContentEditable || active.matches('input, textarea, select'))) return;

    editor.focus();

    const range = document.createRange();
    const selection = window.getSelection();

    range.selectNodeContents(editor);
    range.collapse(false);
    selection?.removeAllRanges();
    selection?.addRange(range);
  });
}

// Read and removed from the page before anything else runs. Only the top frame installs: the
// initialization script can also run in embedded cross-origin frames.
const config = window.__SHIVER__;

delete window.__SHIVER__;

if (config && isTopFrame()) {
  try {
    install(config);
  } catch (error) {
    console.error('[shiver] bridge failed to install', error);
  }
}
