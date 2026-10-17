/**
 * Each DM conversation's latest message, sent or received, and which ones the user closed, from the
 * page's own connection to its server. Sharkord's client keeps its DM list to itself, so the bridge
 * reads it off the socket instead:
 *
 * - the join's answer lists every conversation with its latest time and when the user closed it
 *   (Sharkord 0.0.28 and newer). An older server lists none there and has `dms.get` instead, which
 *   the bridge asks on the same socket once the page has subscribed to new messages (right after
 *   joining);
 * - after that, each new message on the `messages.onNew` subscription, which Sharkord publishes to
 *   everyone who can see the channel, its author included, and each conversation the page hears of
 *   on `dms.onConversationUpdate` (one opened, closed or written in).
 *
 * Sharkord's tRPC client numbers its own requests and ignores an answer to an id it did not send,
 * so the bridge's question, under a string id, is invisible to it. Must be installed before the page
 * opens its socket.
 */

const JOIN_PATH = 'others.joinServer';
const MESSAGE_PATH = 'messages.onNew';
const CONVERSATION_PATH = 'dms.onConversationUpdate';
const DMS_PATH = 'dms.get';
const INSTALLED = Symbol.for('shiver.messages');
/** the closed conversations, on `window` so that every bridge script in the page reads one list */
const CLOSED = Symbol.for('shiver.closedDms');

type Frame = {
  id?: unknown;
  method?: unknown;
  params?: { path?: unknown };
  result?: { type?: unknown; data?: unknown };
};

type Watch = {
  /** a message in a channel, at the server's time in milliseconds */
  onMessage: (channelId: number, at: number) => void;
  /** conversations' latest messages, by channel id: every one after each join, then each update */
  onDmTimes: (times: Map<number, number>) => void;
  /** a conversation was closed, or one closed came back */
  onClosedChanged?: () => void;
};

type ClosedWindow = { [CLOSED]?: Map<number, number> };

/** channel id -> when the user closed it, for the conversations closed since their last message */
const closedMap = () => {
  const holder = window as unknown as ClosedWindow;

  return (holder[CLOSED] ??= new Map());
};

/** The conversations the user closed, which Sharkord's own list leaves out until a message arrives. */
export const closedDms = (): ReadonlySet<number> => {
  try {
    return new Set(closedMap().keys());
  } catch {
    return new Set();
  }
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

/** A conversation list (the join's, `dms.get`'s, or one update) as channel id -> latest message time; entries that do not read are skipped. */
export const readDmTimes = (data: unknown) => {
  const times = new Map<number, number>();

  for (const conversation of Array.isArray(data) ? data : []) {
    const channelId: unknown = conversation?.channelId;
    const at = time(conversation?.lastMessageAt);

    if (Number.isSafeInteger(channelId) && at !== null) times.set(channelId as number, at);
  }

  return times;
};

/** Takes the closed state of each readable conversation in `data`; true when the closed list changed. */
export const noteClosed = (data: unknown, closed: Map<number, number>) => {
  let changed = false;

  for (const conversation of Array.isArray(data) ? data : []) {
    const channelId: unknown = conversation?.channelId;

    if (!Number.isSafeInteger(channelId)) continue;

    const closedAt = time(conversation?.closedAt);
    const isClosed = closedAt !== null && (time(conversation?.lastMessageAt) ?? 0) <= closedAt;

    if (isClosed && closed.get(channelId as number) !== closedAt) {
      closed.set(channelId as number, closedAt);
      changed = true;
    } else if (!isClosed && closed.delete(channelId as number)) {
      changed = true;
    }
  }

  return changed;
};

export function watchDmActivity({ onMessage, onDmTimes, onClosedChanged }: Watch) {
  const native = window.WebSocket as (typeof WebSocket & { [INSTALLED]?: true }) | undefined;

  if (!native || native[INSTALLED]) return;

  native[INSTALLED] = true;

  const send = native.prototype.send;
  const closed = closedMap();
  let asked = 0;

  type SocketState = {
    /** its `messages.onNew` subscription ids */
    messages: Set<unknown>;
    /** its `dms.onConversationUpdate` subscription ids */
    conversations: Set<unknown>;
    /** the page's join, until it is answered */
    join: unknown;
    /** the join listed the conversations, so `dms.get` is not needed */
    listed: boolean;
    /** the bridge's own `dms.get` */
    question: string | null;
  };

  const sockets = new WeakMap<WebSocket, SocketState>();

  const take = (list: unknown) => {
    onDmTimes(readDmTimes(list));

    if (noteClosed(list, closed)) onClosedChanged?.();
  };

  const listen = (socket: WebSocket) => {
    const state: SocketState = {
      messages: new Set(),
      conversations: new Set(),
      join: null,
      listed: false,
      question: null
    };

    sockets.set(socket, state);
    socket.addEventListener('message', (event) => {
      // most frames are none of these; only those that could be are parsed
      if (typeof event.data !== 'string') return;
      if (!event.data.includes('"channelId"') && !(state.join !== null && event.data.includes('"directMessages"'))) return;

      for (const frame of frames(event.data)) {
        if (frame.result?.type !== 'data') continue;

        const data = frame.result.data as Record<string, unknown> | null;

        if (state.join !== null && frame.id === state.join) {
          state.join = null;

          if (Array.isArray(data?.directMessages)) {
            state.listed = true;
            take(data.directMessages);
          }
        } else if (state.question !== null && frame.id === state.question) {
          state.question = null;
          take(data);
        } else if (state.conversations.has(frame.id)) {
          take([data]);
        } else if (state.messages.has(frame.id)) {
          const channelId = data?.channelId;
          const at = time(data?.createdAt);

          if (!Number.isSafeInteger(channelId) || at === null) continue;

          onMessage(channelId as number, at);

          // a message after it was closed brings a conversation back, as in Sharkord's list
          if ((closed.get(channelId as number) ?? Infinity) < at) {
            closed.delete(channelId as number);
            onClosedChanged?.();
          }
        }
      }
    });

    return state;
  };

  native.prototype.send = function (this: WebSocket, data: Parameters<WebSocket['send']>[0]) {
    const result = send.call(this, data);

    if (
      typeof data === 'string' &&
      (data.includes(JOIN_PATH) || data.includes(MESSAGE_PATH) || data.includes(CONVERSATION_PATH))
    ) {
      for (const frame of frames(data)) {
        const path = frame.params?.path;
        const state = () => sockets.get(this) ?? listen(this);

        if (frame.method === 'query' && path === JOIN_PATH) {
          const socket = state();

          // a fresh join (a reconnect) lists everything again
          socket.join = frame.id;
          socket.listed = false;
        } else if (frame.method === 'subscription' && path === CONVERSATION_PATH) {
          state().conversations.add(frame.id);
        } else if (frame.method === 'subscription' && path === MESSAGE_PATH) {
          const socket = state();

          socket.messages.add(frame.id);

          // the page subscribes right after joining; a server whose join listed nothing is older
          // than 0.0.28 and still answers this
          if (!socket.listed && socket.question === null) {
            socket.question = `shiver-dms-${++asked}`;
            send.call(
              this,
              JSON.stringify({ id: socket.question, jsonrpc: '2.0', method: 'query', params: { path: DMS_PATH } })
            );
          }
        }
      }
    }

    return result;
  };
}
