/**
 * Sharkord's desktop api (`window.sharkordDesktop`), which a Sharkord client looks for as it loads
 * and, finding it at the version it speaks, uses instead of the browser: it hands over its
 * notifications whole (the message they are for, or a plugin's own), its connection state and its
 * keyboard shortcuts, and leads where a notification points when told it was clicked. A client of
 * another version, or one from before the api, finds nothing it can use and stays a web page, read
 * as before.
 *
 * Nothing here reaches the core directly: like the rest of the bridge it only queues what the page
 * says, for the core's drain, and takes the core's answers through the bridge's hooks. Everything
 * the page passes is checked and bounded first.
 */

import { ensureStyle } from '../../shared/web/bridge/dom';

/** The api version this bridge speaks; Sharkord's client must ask for exactly this one. */
export const DESKTOP_API_VERSION = 1;

const MAX_TITLE = 300;
const MAX_BODY = 2000;
const MAX_URL = 2048;
const MAX_BINDINGS = 100;
const MAX_PLUGIN_ID = 200;
/** plugin notifications whose click can still be passed on, oldest forgotten first */
const MAX_PLUGIN_TARGETS = 100;
/** how long a click waits for the client (just opened) to start listening for one */
const CLICK_WAIT_MS = 15_000;

export type ConnectionState = 'connecting' | 'connected' | 'signed-out' | 'disconnected';

const CONNECTION_STATES: readonly string[] = ['connecting', 'connected', 'signed-out', 'disconnected'];

export type MessageTarget = {
  type: 'message';
  channelId: number;
  messageId: number;
  parentMessageId?: number;
  isDm: boolean;
};

type PluginTarget = { type: 'plugin'; pluginId: string; data?: unknown };

type ClickTarget = MessageTarget | PluginTarget;

/** A notification as the client hands it over, checked. */
export type DesktopNotification = {
  title: string;
  body: string;
  icon: string | null;
  /** the message it is for, if it is one */
  message: MessageTarget | null;
  /** the number a plugin's notification is remembered by, for its click */
  pluginRef: number | null;
};

export type HotkeyBinding = { actionId: string; combo: string };

export type HotkeyError = { actionId: string; reason: 'unsupported' | 'taken' };

type DesktopSetup = {
  notify: (notification: DesktopNotification) => void;
  openSettings: () => void;
  /** answered once the core has registered them */
  setHotkeys: (bindings: HotkeyBinding[]) => Promise<HotkeyError[]>;
};

type Listener<T> = (value: T) => void;

declare global {
  interface Window {
    sharkordDesktop?: { version: number; connect: (clientVersion: number) => unknown };
  }
}

const text = (value: unknown, max: number) => (typeof value === 'string' ? value.slice(0, max) : '');
const id = (value: unknown) => (Number.isSafeInteger(value) && (value as number) > 0 ? (value as number) : null);

const clickListeners = new Set<Listener<ClickTarget>>();
const hotkeyListeners = new Set<(actionId: string, phase: 'down') => void>();
const pluginTargets = new Map<number, PluginTarget>();
let nextPluginRef = 1;
let connected = false;
let connection: ConnectionState | null = null;

/** Whether the page's client took the api, so its notifications and shortcuts come through it. */
export const isDesktopClient = () => connected;

/** The client's own word on its connection, once it has given one. */
export const connectionState = () => connection;

const call = <T>(listeners: Set<Listener<T>>, value: T) => {
  for (const listener of listeners) {
    try {
      listener(value);
    } catch (error) {
      console.error('[shiver] a page listener failed', error);
    }
  }
};

const subscribe = <L>(listeners: Set<L>, listener: unknown) => {
  if (typeof listener !== 'function') return () => undefined;

  const added = listener as L;

  listeners.add(added);

  return () => void listeners.delete(added);
};

/** A notification's target as the client gave it, or null when it is not one this can lead to. */
export function readNotification(raw: unknown): DesktopNotification | null {
  if (!raw || typeof raw !== 'object') return null;

  const { title, body, icon, target } = raw as Record<string, unknown>;
  const notification: DesktopNotification = {
    title: text(title, MAX_TITLE),
    body: text(body, MAX_BODY),
    icon: typeof icon === 'string' && icon.length <= MAX_URL ? icon : null,
    message: null,
    pluginRef: null
  };

  if (!notification.title) return null;

  const fields = (target && typeof target === 'object' ? target : {}) as Record<string, unknown>;

  if (fields.type === 'message') {
    const channelId = id(fields.channelId);
    const messageId = id(fields.messageId);

    if (channelId !== null && messageId !== null) {
      notification.message = {
        type: 'message',
        channelId,
        messageId,
        ...(id(fields.parentMessageId) !== null ? { parentMessageId: id(fields.parentMessageId) as number } : {}),
        isDm: fields.isDm === true
      };
    }
  } else if (fields.type === 'plugin' && typeof fields.pluginId === 'string' && fields.pluginId.length <= MAX_PLUGIN_ID) {
    notification.pluginRef = nextPluginRef++;
    pluginTargets.set(notification.pluginRef, { type: 'plugin', pluginId: fields.pluginId, data: fields.data });

    for (const ref of pluginTargets.keys()) {
      if (pluginTargets.size <= MAX_PLUGIN_TARGETS) break;

      pluginTargets.delete(ref);
    }
  }

  return notification;
}

/** The bindings the client asked for, checked; anything malformed is left out. */
export function readBindings(raw: unknown): HotkeyBinding[] {
  return (Array.isArray(raw) ? raw : [])
    .filter(
      (binding): binding is HotkeyBinding =>
        typeof binding?.actionId === 'string' &&
        typeof binding?.combo === 'string' &&
        binding.actionId.length > 0 &&
        binding.actionId.length <= 200 &&
        binding.combo.length > 0 &&
        binding.combo.length <= 100
    )
    .slice(0, MAX_BINDINGS)
    .map(({ actionId, combo }) => ({ actionId, combo }));
}

/** The core's answer to a set of bindings, checked. */
export function readHotkeyErrors(raw: unknown): HotkeyError[] {
  return (Array.isArray(raw) ? raw : []).filter(
    (error): error is HotkeyError =>
      typeof error?.actionId === 'string' && (error?.reason === 'unsupported' || error?.reason === 'taken')
  );
}

/**
 * Tells the client one of its shortcuts was pressed. Only presses: the api offers no key releases
 * (`keyUp: false`), as Sharkord's own desktop app on Windows, so the client keeps push-to-talk to
 * the page.
 */
export const pressHotkey = (actionId: unknown) => {
  if (typeof actionId !== 'string') return;

  for (const listener of hotkeyListeners) {
    try {
      listener(actionId, 'down');
    } catch (error) {
      console.error('[shiver] a page listener failed', error);
    }
  }
};

/**
 * Leads the page where a clicked notification points, through the client's own handling (the
 * message, in its thread if it is a reply; or the plugin's). Waits a while for a client that is
 * still starting to listen. Returns false when there is nothing to lead to this way: the page is
 * not a desktop client, the notification named no message, or a plugin's was forgotten (a reload).
 */
export function openNotification(
  channelId: number | null,
  target: { messageId?: unknown; threadId?: unknown; pluginRef?: unknown } | null,
  isDm: (channelId: number) => boolean,
  fallback: () => void
) {
  if (!connected || !target) return false;

  let click: ClickTarget | null = null;
  const pluginRef = id(target.pluginRef);
  const messageId = id(target.messageId);

  if (pluginRef !== null) {
    click = pluginTargets.get(pluginRef) ?? null;
  } else if (channelId !== null && messageId !== null) {
    const threadId = id(target.threadId);

    click = {
      type: 'message',
      channelId,
      messageId,
      ...(threadId !== null ? { parentMessageId: threadId } : {}),
      isDm: isDm(channelId)
    };
  }

  if (!click) return false;

  const chosen = click;
  const deadline = Date.now() + CLICK_WAIT_MS;
  const attempt = () => {
    if (clickListeners.size > 0) {
      call(clickListeners, chosen);

      return true;
    }

    if (Date.now() <= deadline) return false;

    fallback();

    return true;
  };

  if (attempt()) return true;

  const timer = window.setInterval(() => {
    if (attempt()) window.clearInterval(timer);
  }, 250);

  return true;
}

/**
 * Puts the api on `window` for the client to find. Shiver's rail is always shown, so the client's
 * button for hiding a desktop app's server list is hidden rather than left doing nothing.
 */
export function installDesktopApi(setup: DesktopSetup) {
  const api = {
    getServerListVisible: () => Promise.resolve(true),
    onServerListVisibleChange: () => () => undefined,
    toggleServerList: () => Promise.resolve(),
    openSettings: () => {
      setup.openSettings();

      return Promise.resolve();
    },
    // Shiver counts unread itself, without the muted channels Sharkord's count includes
    setUnreadState: () => undefined,
    setConnectionState: (state: unknown) => {
      if (typeof state === 'string' && CONNECTION_STATES.includes(state)) connection = state as ConnectionState;
    },
    // Shiver follows voice itself, from the page (`voice.ts`)
    setVoiceActive: () => undefined,
    // a virtual microphone for system audio is Linux's (venmic); WebView2 captures it with the screen
    startSystemAudio: () => Promise.resolve(false),
    stopSystemAudio: () => Promise.resolve(),
    showNotification: (raw: unknown) => {
      const notification = readNotification(raw);

      if (notification) setup.notify(notification);
    },
    setHotkeys: (raw: unknown) => setup.setHotkeys(readBindings(raw)),
    getHotkeyCapabilities: () => Promise.resolve({ keyUp: false, systemManaged: false }),
    onHotkey: (listener: unknown) => subscribe(hotkeyListeners, listener),
    // what could not be registered is answered to `setHotkeys`, and nothing is reported later
    onHotkeyErrors: () => () => undefined,
    onNotificationClick: (listener: unknown) => subscribe(clickListeners, listener)
  };

  const entry = Object.freeze({
    version: DESKTOP_API_VERSION,
    connect: (clientVersion: unknown) => {
      if (clientVersion !== DESKTOP_API_VERSION) return undefined;

      if (!connected) {
        connected = true;
        hideServerListToggle();
      }

      return api;
    }
  });

  try {
    Object.defineProperty(window, 'sharkordDesktop', { configurable: true, value: entry });
  } catch {
    // already taken: the page is a web page, read as before
  }
}

/**
 * The client's server-list button, the first in its top bar's left cell (a desktop app's only). The
 * top bar is found by its column template, its own in every version (it is `lg:grid` before
 * Sharkord 0.0.29 and `grid` in a desktop app since).
 */
function hideServerListToggle() {
  ensureStyle('shiver-desktop-api').textContent =
    '[class~="grid-cols-[1fr_minmax(320px,1.4fr)_1fr]"] > div:first-child > button:first-child { display: none !important; }';
}
