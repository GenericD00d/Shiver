import { expect, test } from 'bun:test';

import { closedDms, noteClosed, readDmTimes, watchDmActivity } from './messages';

/** Just enough of a WebSocket: what is sent is kept, and the test plays the server. */
class FakeSocket extends EventTarget {
  sent: string[] = [];

  send(data: string) {
    this.sent.push(data);
  }

  receive(frame: unknown) {
    this.dispatchEvent(Object.assign(new Event('message'), { data: JSON.stringify(frame) }));
  }
}

test("dms.get's answer reads as each conversation's latest time, skipping what does not read", () => {
  expect([
    ...readDmTimes([
      { channelId: 3, userId: 2, unreadCount: 0, lastMessageAt: 5_000 },
      { channelId: 4, lastMessageAt: 'soon' },
      { channelId: 'x', lastMessageAt: 1 },
      null
    ])
  ]).toEqual([[3, 5_000]]);
  expect(readDmTimes({ not: 'a list' }).size).toBe(0);
});

test('a conversation is closed while nothing was said after it was closed', () => {
  const closed = new Map<number, number>();

  expect(
    noteClosed(
      [
        { channelId: 1, lastMessageAt: 100, closedAt: null },
        { channelId: 2, lastMessageAt: 100, closedAt: 200 },
        { channelId: 3, lastMessageAt: 300, closedAt: 200 },
        { channelId: 4, closedAt: 200 },
        { channelId: 'x', closedAt: 1 }
      ],
      closed
    )
  ).toBe(true);
  expect([...closed]).toEqual([
    [2, 200],
    [4, 200]
  ]);
  // the same again changes nothing; a message after the close reopens it
  expect(noteClosed([{ channelId: 2, lastMessageAt: 100, closedAt: 200 }], closed)).toBe(false);
  expect(noteClosed([{ channelId: 2, lastMessageAt: 250, closedAt: 200 }], closed)).toBe(true);
  expect([...closed.keys()]).toEqual([4]);
});

/** Sharkord 0.0.28: the join lists the conversations, and there is no `dms.get` to ask. */
test("a join that lists the conversations is read, and nothing is asked of the server", () => {
  // a class of its own, since the bridge patches the one it finds once
  class JoinedSocket extends FakeSocket {}

  (globalThis as { window?: unknown }).window = { WebSocket: JoinedSocket };

  const messages: [number, number][] = [];
  const answers: [number, number][][] = [];
  let closedChanges = 0;

  watchDmActivity({
    onMessage: (channelId, at) => messages.push([channelId, at]),
    onDmTimes: (times) => answers.push([...times]),
    onClosedChanged: () => closedChanges++
  });

  const socket = new JoinedSocket();

  socket.send(JSON.stringify({ id: 1, method: 'query', params: { path: 'others.joinServer' } }));
  socket.receive({
    id: 1,
    result: {
      type: 'data',
      data: {
        channels: [],
        directMessages: [
          { channelId: 3, userId: 2, lastMessageAt: 1_000, closedAt: null },
          { channelId: 5, userId: 4, lastMessageAt: 1_000, closedAt: 2_000 }
        ]
      }
    }
  });
  socket.send(JSON.stringify({ id: 7, method: 'subscription', params: { path: 'messages.onNew' } }));
  socket.send(JSON.stringify({ id: 9, method: 'subscription', params: { path: 'dms.onConversationUpdate' } }));

  expect(socket.sent.some((frame) => frame.includes('dms.get'))).toBe(false);
  expect(answers).toEqual([
    [
      [3, 1_000],
      [5, 1_000]
    ]
  ]);
  expect([...closedDms()]).toEqual([5]);
  expect(closedChanges).toBe(1);

  // closed on another device, then written in again
  socket.receive({ id: 9, result: { type: 'data', data: { channelId: 3, userId: 2, lastMessageAt: 1_000, closedAt: 1_500 } } });
  expect([...closedDms()].sort()).toEqual([3, 5]);
  socket.receive({ id: 7, result: { type: 'data', data: { channelId: 3, createdAt: 3_000 } } });
  expect([...closedDms()]).toEqual([5]);
  expect(messages).toEqual([[3, 3_000]]);
  expect(closedChanges).toBe(3);
});

test("an older server's page is asked for DM times once joined, then followed for every message", () => {
  (globalThis as { window?: unknown }).window = { WebSocket: FakeSocket };

  const messages: [number, number][] = [];
  const answers: [number, number][][] = [];
  const watch = {
    onMessage: (channelId: number, at: number) => messages.push([channelId, at]),
    onDmTimes: (times: Map<number, number>) => answers.push([...times])
  };

  watchDmActivity(watch);
  // a second install is ignored rather than doubling everything
  watchDmActivity(watch);

  const socket = new FakeSocket();

  // before the join, the page asks nothing of interest and the bridge asks nothing
  socket.send(JSON.stringify({ id: 1, method: 'query', params: { path: 'others.joinServer' } }));
  expect(socket.sent.length).toBe(1);

  // after it, the page subscribes, and the bridge asks once, under an id the page never uses
  socket.send(JSON.stringify({ id: 7, method: 'subscription', params: { path: 'messages.onNew' } }));
  socket.send(JSON.stringify({ id: 8, method: 'subscription', params: { path: 'messages.onUpdate' } }));

  const questions = socket.sent.map((frame) => JSON.parse(frame)).filter((frame) => frame.params?.path === 'dms.get');

  expect(questions).toEqual([{ id: 'shiver-dms-1', jsonrpc: '2.0', method: 'query', params: { path: 'dms.get' } }]);

  socket.receive({ id: 'shiver-dms-1', result: { type: 'data', data: [{ channelId: 3, lastMessageAt: 1_000 }] } });
  socket.receive({ id: 7, result: { type: 'data', data: { channelId: 3, createdAt: 2_000, userId: 1 } } });
  socket.receive({ id: 8, result: { type: 'data', data: { channelId: 4, createdAt: 3_000 } } });
  socket.receive({ id: 7, result: { type: 'data', data: { channelId: '3', createdAt: 4_000 } } });
  socket.dispatchEvent(Object.assign(new Event('message'), { data: '{"channelId": not json' }));

  expect(answers).toEqual([[[3, 1_000]]]);
  expect(messages).toEqual([[3, 2_000]]);

  // a new connection (a reconnect) joins again, and is asked again
  const again = new FakeSocket();

  again.send(JSON.stringify([{ id: 1, method: 'subscription', params: { path: 'messages.onNew' } }]));
  expect(again.sent.some((frame) => frame.includes('"shiver-dms-2"'))).toBe(true);
});
