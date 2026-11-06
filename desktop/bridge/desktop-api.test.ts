import { expect, test } from 'bun:test';

import {
  DESKTOP_API_VERSION,
  desktopMismatch,
  type DesktopNotification,
  type HotkeyBinding,
  installDesktopApi,
  isDesktopClient,
  openNotification,
  pressHotkey,
  readBindings,
  readHotkeyErrors,
  readNotification
} from './desktop-api';

type Api = {
  showNotification: (raw: unknown) => void;
  setHotkeys: (raw: unknown) => Promise<unknown>;
  onHotkey: (listener: (actionId: string, phase: string) => void) => () => void;
  onHotkeyErrors: (listener: (errors: unknown[]) => void) => () => void;
  getHotkeyCapabilities: () => Promise<{ keyUp: boolean; systemManaged: boolean }>;
  setVoiceActive: (active: boolean) => void;
  onNotificationClick: (listener: (target: unknown) => void) => () => void;
  openSettings: () => Promise<void>;
  getServerListVisible: () => Promise<boolean>;
  startSystemAudio: () => Promise<boolean>;
};

test('a notification is read with the message it is for, and anything malformed is dropped', () => {
  expect(
    readNotification({
      title: 'Ana in #general',
      body: 'hi',
      icon: 'https://chat.example/a.png',
      target: { type: 'message', channelId: 10, messageId: 7, parentMessageId: 5, isDm: false }
    })
  ).toEqual({
    title: 'Ana in #general',
    body: 'hi',
    icon: 'https://chat.example/a.png',
    message: { type: 'message', channelId: 10, messageId: 7, parentMessageId: 5, isDm: false },
    pluginRef: null
  });

  // no thread, and a message id that is not one
  expect(readNotification({ title: 't', body: 'b', target: { type: 'message', channelId: 10, messageId: 7, isDm: true } })?.message).toEqual({
    type: 'message',
    channelId: 10,
    messageId: 7,
    isDm: true
  });
  expect(readNotification({ title: 't', target: { type: 'message', channelId: 10, messageId: -1 } })?.message).toBeNull();
  expect(readNotification({ title: 't', body: 3, icon: 4 })).toEqual({ title: 't', body: '', icon: null, message: null, pluginRef: null });

  expect(readNotification({ body: 'no title' })).toBeNull();
  expect(readNotification('a string')).toBeNull();
  expect(readNotification({ title: 'x'.repeat(5000) })?.title.length).toBe(300);
});

test("a plugin's notification is remembered by a number, for its click", () => {
  const first = readNotification({ title: 'Builds: done', target: { type: 'plugin', pluginId: 'ci', data: { id: 4 } } });
  const second = readNotification({ title: 'Builds: done', target: { type: 'plugin', pluginId: 'ci' } });

  expect(first?.pluginRef).toBeNumber();
  expect(second?.pluginRef).toBe((first?.pluginRef ?? 0) + 1);
  expect(readNotification({ title: 't', target: { type: 'plugin', pluginId: 5 } })?.pluginRef).toBeNull();
});

test('bindings and the core\'s answer are checked', () => {
  expect(
    readBindings([
      { actionId: 'mute', combo: 'ctrl+KeyM' },
      { actionId: '', combo: 'KeyA' },
      { actionId: 'x', combo: 'k'.repeat(101) },
      { actionId: 'deafen', combo: 'ctrl+KeyD', extra: true },
      'nonsense'
    ])
  ).toEqual([
    { actionId: 'mute', combo: 'ctrl+KeyM' },
    { actionId: 'deafen', combo: 'ctrl+KeyD' }
  ]);
  expect(readBindings('nonsense')).toEqual([]);
  expect(readBindings(Array.from({ length: 150 }, (_, n) => ({ actionId: `a${n}`, combo: 'KeyA' }))).length).toBe(100);

  expect(
    readHotkeyErrors([
      { actionId: 'mute', reason: 'taken' },
      { actionId: 'x', reason: 'because' },
      { reason: 'unsupported' }
    ])
  ).toEqual([{ actionId: 'mute', reason: 'taken' }]);
});

test('a client of the version spoken connects, takes the page\'s notifications and shortcuts, and is led on a click', async () => {
  class FakeStyle {
    id = '';
    textContent = '';
  }

  const styles = new Map<string, FakeStyle>();

  (globalThis as { HTMLStyleElement?: unknown }).HTMLStyleElement = FakeStyle;
  (globalThis as { document?: unknown }).document = {
    getElementById: (id: string) => styles.get(id) ?? null,
    createElement: () => new FakeStyle(),
    documentElement: { appendChild: (style: FakeStyle) => styles.set(style.id, style) }
  };
  (globalThis as { window?: unknown }).window = globalThis;

  const notified: DesktopNotification[] = [];
  const asked: HotkeyBinding[][] = [];

  installDesktopApi({
    notify: (notification) => notified.push(notification),
    setHotkeys: (bindings) => {
      asked.push(bindings);

      return Promise.resolve([{ actionId: 'mute', reason: 'taken' }]);
    }
  });

  const entry = (globalThis as { sharkordDesktop?: { version: number; connect: (version: unknown) => unknown } }).sharkordDesktop;

  expect(entry?.version).toBe(DESKTOP_API_VERSION);
  expect(desktopMismatch()).toBeNull();
  expect(entry?.connect(DESKTOP_API_VERSION + 1)).toBeUndefined();
  expect(desktopMismatch()).toBe('app-outdated');
  expect(entry?.connect(DESKTOP_API_VERSION - 1)).toBeUndefined();
  expect(desktopMismatch()).toBe('server-outdated');
  expect(entry?.connect('nonsense')).toBeUndefined();
  expect(desktopMismatch()).toBe('server-outdated');
  expect(isDesktopClient()).toBe(false);
  // nothing to lead with before a client connects
  expect(openNotification(10, { messageId: 7 }, () => false, () => undefined)).toBe(false);

  const api = entry?.connect(DESKTOP_API_VERSION) as Api;

  expect(isDesktopClient()).toBe(true);
  expect(desktopMismatch()).toBeNull();
  expect(styles.get('shiver-desktop-api')?.textContent).toContain('display: none');
  expect(await api.getServerListVisible()).toBe(true);
  expect(await api.startSystemAudio()).toBe(false);

  api.showNotification({ title: 'Ana (DM)', body: 'hi', target: { type: 'message', channelId: 20, messageId: 3, isDm: true } });
  api.showNotification({ nonsense: true });
  expect(notified.map((notification) => notification.message?.messageId)).toEqual([3]);

  // nothing to open: the button asking is hidden
  expect(await api.openSettings()).toBeUndefined();

  expect(await api.setHotkeys([{ actionId: 'mute', combo: 'ctrl+KeyM' }, { bad: 1 }])).toEqual([{ actionId: 'mute', reason: 'taken' }]);
  expect(asked).toEqual([[{ actionId: 'mute', combo: 'ctrl+KeyM' }]]);

  // as Sharkord 0.0.29's client starts: voice, then the shortcuts' capabilities and errors
  api.setVoiceActive(true);
  expect(await api.getHotkeyCapabilities()).toEqual({ keyUp: true, systemManaged: false });

  const stopErrors = api.onHotkeyErrors(() => undefined);

  expect(typeof stopErrors).toBe('function');
  stopErrors();

  const pressed: string[] = [];
  const stop = api.onHotkey((actionId, phase) => pressed.push(`${actionId} ${phase}`));

  pressHotkey('mute');
  pressHotkey('push-to-talk', 'down');
  pressHotkey('push-to-talk', 'up');
  pressHotkey(5);
  stop();
  pressHotkey('mute');
  expect(pressed).toEqual(['mute down', 'push-to-talk down', 'push-to-talk up']);

  const clicked: unknown[] = [];

  api.onNotificationClick((target) => clicked.push(target));

  let fellBack = 0;

  expect(openNotification(10, { messageId: 7, threadId: 5 }, (id) => id === 20, () => void (fellBack += 1))).toBe(true);
  expect(openNotification(20, { messageId: 8 }, (id) => id === 20, () => void (fellBack += 1))).toBe(true);
  // a channel alone, or a plugin's notification the page no longer knows: only the channel opens
  expect(openNotification(10, { messageId: null }, () => false, () => undefined)).toBe(false);
  expect(openNotification(null, { pluginRef: 999_999 }, () => false, () => undefined)).toBe(false);

  const plugin = readNotification({ title: 'CI: green', target: { type: 'plugin', pluginId: 'ci', data: { id: 4 } } });

  expect(openNotification(null, { pluginRef: plugin?.pluginRef }, () => false, () => undefined)).toBe(true);

  expect(clicked).toEqual([
    { type: 'message', channelId: 10, messageId: 7, parentMessageId: 5, isDm: false },
    { type: 'message', channelId: 20, messageId: 8, isDm: true },
    { type: 'plugin', pluginId: 'ci', data: { id: 4 } }
  ]);
  expect(fellBack).toBe(0);
});
