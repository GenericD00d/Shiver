import { expect, test } from 'bun:test';

import { watchNewMessages } from './messages';

/** Just enough of a WebSocket: what is sent is kept, and the test plays the server. */
class FakeSocket extends EventTarget {
  sent: unknown[] = [];

  send(data: unknown) {
    this.sent.push(data);
  }

  receive(frame: unknown) {
    this.dispatchEvent(Object.assign(new Event('message'), { data: JSON.stringify(frame) }));
  }
}

test("the page's own message subscription is followed, sent and received alike, and nothing else", () => {
  (globalThis as { window?: unknown }).window = { WebSocket: FakeSocket };

  const heard: [number, number][] = [];

  watchNewMessages((channelId, at) => heard.push([channelId, at]));
  // a second install is ignored rather than doubling every message
  watchNewMessages((channelId, at) => heard.push([channelId, at]));

  const socket = new FakeSocket();

  socket.send(JSON.stringify({ id: 7, method: 'subscription', params: { path: 'messages.onNew' } }));
  socket.send(JSON.stringify({ id: 8, method: 'subscription', params: { path: 'messages.onUpdate' } }));

  socket.receive({ id: 7, result: { type: 'data', data: { channelId: 3, createdAt: 1_000, userId: 1 } } });
  socket.receive({ id: 8, result: { type: 'data', data: { channelId: 4, createdAt: 2_000 } } });
  socket.receive({ id: 7, result: { type: 'started' } });
  socket.receive({ id: 7, result: { type: 'data', data: { channelId: '3', createdAt: 3_000 } } });
  socket.dispatchEvent(Object.assign(new Event('message'), { data: '{"channelId": not json' }));

  expect(heard).toEqual([[3, 1_000]]);
  // what the page sends still goes out unchanged
  expect(socket.sent.length).toBe(2);
});
