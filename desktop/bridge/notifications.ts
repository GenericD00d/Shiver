/** Sharkord's notifications, taken over: queued for the core instead of shown, and its ping silenced. */

import { findDmChannelIdByUserName, type SharkordState } from '../../shared/web/bridge/sharkord';
import type { QueuedNotification } from './types';

/** What a mention of the user reads as: their name, and each role they hold (role mentions, 0.0.27). */
export const ownMentionNames = (state: SharkordState) => {
  const own = (state.users ?? []).find((user) => user.id === state.ownUserId);
  const held = new Set(own?.roleIds ?? []);

  return [own?.name ?? '', ...(state.roles ?? []).filter((role) => held.has(role.id)).map((role) => role.name)].filter(
    (name) => name !== ''
  );
};

/** Whether a notification's text mentions any of `names` (`@name`, as Sharkord writes a mention in text). */
export const mentionsAny = (text: string, names: readonly string[]) => names.some((name) => text.includes(`@${name}`));

/** Sharkord's incoming-message tone: a single 600 Hz sine (`sfxMessageReceived`). */
const MESSAGE_PING_HZ = 600;

/**
 * Swallows Sharkord's incoming-message ping, which it plays for every message regardless of
 * Shiver's mutes; Shiver plays its own for the notifications that survive them. Only that tone is
 * dropped (matched on the frequency Sharkord requested): every other sound still plays.
 */
export function silenceMessagePing() {
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
 * Replaces `window.Notification` so the page's notifications go to `handle` instead of the OS.
 * Claims permission, since Sharkord composes nothing without it.
 */
export function installNotificationWrapper(handle: (title: string, options?: NotificationOptions) => void) {
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

/**
 * Retries the channel of queued DM notifications captured before the store had users and DM
 * channels; a notification without a channel could never be cleared by reading.
 */
export function resolveDmChannels(queue: QueuedNotification[], state: SharkordState) {
  for (const queued of queue) {
    if (queued.isDm && queued.channelId === null) queued.channelId = findDmChannelIdByUserName(state, queued.author);
  }

  return queue;
}
