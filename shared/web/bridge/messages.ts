/**
 * New messages as the page's own connection hears them, sent and received alike: Sharkord publishes
 * each one to everyone who can see its channel, its author included, on the `messages.onNew`
 * subscription. The page's store does not keep them where a plugin can read, so the bridge watches
 * the subscription's frames instead. Must be installed before the page opens its socket.
 */

const MESSAGE_PATH = 'messages.onNew';
const INSTALLED = Symbol.for('shiver.messages');

type Frame = {
  id?: unknown;
  method?: unknown;
  params?: { path?: unknown };
  result?: { type?: unknown; data?: { channelId?: unknown; createdAt?: unknown } };
};

const frames = (text: string): Frame[] => {
  try {
    const parsed: unknown = JSON.parse(text);

    return (Array.isArray(parsed) ? parsed : [parsed]).filter(
      (frame): frame is Frame => !!frame && typeof frame === 'object'
    );
  } catch {
    return [];
  }
};

/** Calls `onMessage` with each new message's channel and time (the server's, in milliseconds). */
export function watchNewMessages(onMessage: (channelId: number, at: number) => void) {
  const native = window.WebSocket as (typeof WebSocket & { [INSTALLED]?: true }) | undefined;

  if (!native || native[INSTALLED]) return;

  native[INSTALLED] = true;

  /** each socket's `messages.onNew` subscription ids, learned from what the page sends */
  const subscriptions = new WeakMap<WebSocket, Set<unknown>>();
  const send = native.prototype.send;

  const listen = (socket: WebSocket) => {
    const ids = new Set<unknown>();

    subscriptions.set(socket, ids);
    socket.addEventListener('message', (event) => {
      // most frames are not this subscription's; only those are parsed
      if (ids.size === 0 || typeof event.data !== 'string' || !event.data.includes('"channelId"')) return;

      for (const frame of frames(event.data)) {
        const data = frame.result?.type === 'data' && ids.has(frame.id) ? frame.result.data : undefined;
        const channelId = data?.channelId;
        const at = data?.createdAt;

        if (Number.isSafeInteger(channelId) && typeof at === 'number' && Number.isFinite(at)) {
          onMessage(channelId as number, at);
        }
      }
    });

    return ids;
  };

  native.prototype.send = function (this: WebSocket, data: Parameters<WebSocket['send']>[0]) {
    if (typeof data === 'string' && data.includes(MESSAGE_PATH)) {
      for (const frame of frames(data)) {
        if (frame.method === 'subscription' && frame.params?.path === MESSAGE_PATH) {
          (subscriptions.get(this) ?? listen(this)).add(frame.id);
        }
      }
    }

    return send.call(this, data);
  };
}
