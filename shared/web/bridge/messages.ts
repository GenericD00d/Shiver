/**
 * Each DM conversation's latest message, sent or received, from the page's own connection to its
 * server. Sharkord's client keeps its DM list's times to that component, so the bridge reads them
 * off the socket instead:
 *
 * - once the page has joined (it subscribes to new messages right after), it asks `dms.get` on
 *   the same socket, which answers every conversation's latest time, as Sharkord's own list does;
 * - after that, each new message on the `messages.onNew` subscription, which Sharkord publishes to
 *   everyone who can see the channel, its author included.
 *
 * Sharkord's tRPC client numbers its own requests and ignores an answer to an id it did not send,
 * so the bridge's question, under a string id, is invisible to it. Must be installed before the page
 * opens its socket.
 */

const MESSAGE_PATH = 'messages.onNew';
const DMS_PATH = 'dms.get';
const INSTALLED = Symbol.for('shiver.messages');

type Frame = {
  id?: unknown;
  method?: unknown;
  params?: { path?: unknown };
  result?: { type?: unknown; data?: unknown };
};

type Watch = {
  /** a message in a channel, at the server's time in milliseconds */
  onMessage: (channelId: number, at: number) => void;
  /** every conversation's latest message, by channel id, as `dms.get` answers after each join */
  onDmTimes: (times: Map<number, number>) => void;
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

const time = (value: unknown) => (typeof value === 'number' && Number.isFinite(value) ? value : null);

/** `dms.get`'s answer as channel id -> latest message time; entries that do not read are skipped. */
export const readDmTimes = (data: unknown) => {
  const times = new Map<number, number>();

  for (const conversation of Array.isArray(data) ? data : []) {
    const channelId: unknown = conversation?.channelId;
    const at = time(conversation?.lastMessageAt);

    if (Number.isSafeInteger(channelId) && at !== null) times.set(channelId as number, at);
  }

  return times;
};

export function watchDmActivity({ onMessage, onDmTimes }: Watch) {
  const native = window.WebSocket as (typeof WebSocket & { [INSTALLED]?: true }) | undefined;

  if (!native || native[INSTALLED]) return;

  native[INSTALLED] = true;

  const send = native.prototype.send;
  let asked = 0;

  /** per socket: its `messages.onNew` subscription ids, and the id of the bridge's own question */
  const sockets = new WeakMap<WebSocket, { subscriptions: Set<unknown>; question: string | null }>();

  const listen = (socket: WebSocket) => {
    const state = { subscriptions: new Set<unknown>(), question: null as string | null };

    sockets.set(socket, state);
    socket.addEventListener('message', (event) => {
      // most frames are neither; only those that could be are parsed
      if (typeof event.data !== 'string' || !event.data.includes('"channelId"')) return;

      for (const frame of frames(event.data)) {
        if (frame.result?.type !== 'data') continue;

        if (state.question !== null && frame.id === state.question) {
          state.question = null;
          onDmTimes(readDmTimes(frame.result.data));
        } else if (state.subscriptions.has(frame.id)) {
          const data = frame.result.data as { channelId?: unknown; createdAt?: unknown } | null;
          const at = time(data?.createdAt);

          if (Number.isSafeInteger(data?.channelId) && at !== null) onMessage(data!.channelId as number, at);
        }
      }
    });

    return state;
  };

  native.prototype.send = function (this: WebSocket, data: Parameters<WebSocket['send']>[0]) {
    const result = send.call(this, data);

    if (typeof data === 'string' && data.includes(MESSAGE_PATH)) {
      for (const frame of frames(data)) {
        if (frame.method !== 'subscription' || frame.params?.path !== MESSAGE_PATH) continue;

        const state = sockets.get(this) ?? listen(this);

        state.subscriptions.add(frame.id);

        // the page subscribes right after joining, which is when the server answers this
        if (state.question === null) {
          state.question = `shiver-dms-${++asked}`;
          send.call(this, JSON.stringify({ id: state.question, jsonrpc: '2.0', method: 'query', params: { path: DMS_PATH } }));
        }
      }
    }

    return result;
  };
}
