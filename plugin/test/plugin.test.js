import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { splitName, uniqueName } from '../server/files.js';
import { adoptOldStore, primeFromUserRows } from '../server/index.js';
import {
  createPush,
  deliver,
  endpointsFrom,
  isPrivateAddress,
  normaliseEndpoint,
  REFUSED,
  vetEndpoint
} from '../server/push.js';
import { createLimiter, createRows } from '../server/rows.js';
import { floorFrom, mutedFrom } from '../server/settings.js';
import { createStatuses, statusFrom } from '../server/status.js';

/* ── a fake host ── */

const fakeCtx = (rows = {}) => {
  const data = new Map(Object.entries(rows).map(([id, row]) => [Number(id), row]));
  const pushed = [];
  const ctx = {
    data,
    pushed,
    failReads: new Set(),
    logger: { log: () => {}, debug: () => {} },
    userData: {
      get: async (id) => {
        if (ctx.failReads.has(id)) throw new Error('database unavailable');

        await new Promise((resolve) => setImmediate(resolve));

        return data.has(id) ? structuredClone(data.get(id)) : null;
      },
      set: async (id, row) => {
        await new Promise((resolve) => setImmediate(resolve));
        data.set(id, structuredClone(row));
      }
    },
    permissions: { userCanInChannel: async () => true },
    push: { toAll: (payload) => pushed.push(payload) },
    users: { list: async () => [...data.keys()].map((id) => ({ id })) }
  };

  return ctx;
};

const publicLookup = async () => [{ address: '93.184.216.34', family: 4 }];

/* ── addresses ── */

test('private and special addresses are refused in every spelling', () => {
  for (const address of [
    '127.0.0.1',
    '10.1.2.3',
    '172.16.0.1',
    '192.168.1.1',
    '169.254.169.254',
    '100.64.0.1',
    '0.0.0.0',
    '192.88.99.1',
    '224.0.0.1',
    '255.255.255.255',
    '::1',
    '::',
    '::ffff:7f00:1',
    '::ffff:127.0.0.1',
    '::ffff:0:7f00:1',
    '64:ff9b::7f00:1',
    '64:ff9b:1::1',
    '2002:7f00:1::',
    '2002:c0a8:101::1',
    '2001:0:4136:e378::1',
    '2001:db8::1',
    'fc00::1',
    'fd12:3456::1',
    'fe80::1',
    'febf::1',
    'fec0::1',
    'ff02::1',
    '100::1',
    '[::1]',
    'not-an-address'
  ]) {
    assert.equal(isPrivateAddress(address), true, address);
  }
});

test('public addresses are allowed', () => {
  for (const address of ['93.184.216.34', '1.1.1.1', '2606:4700:4700::1111', '2002:5db8:d822::1']) {
    assert.equal(isPrivateAddress(address), false, address);
  }
});

test('endpoints are canonicalised once, so the stored string is the checked one', () => {
  assert.equal(normaliseEndpoint('HTTPS://Ntfy.Example.com/up1#x'), 'https://ntfy.example.com/up1');
  assert.equal(normaliseEndpoint('http://ntfy.example.com/up1'), null);
  assert.equal(normaliseEndpoint('https://user:pw@ntfy.example.com/'), null);
  assert.equal(normaliseEndpoint({ toString: () => 'https://x.example/' }), null);
  assert.equal(normaliseEndpoint(`https://x.example/${'a'.repeat(600)}`), null);
  assert.deepEqual(endpointsFrom(['HTTPS://a.example/1', 'https://a.example/1', 7]), ['https://a.example/1']);
});

test('a hostname is refused if any address it resolves to is private', async () => {
  const mixed = async () => [
    { address: '93.184.216.34', family: 4 },
    { address: '::1', family: 6 }
  ];

  assert.equal((await vetEndpoint('https://relay.example/x', mixed)).ok, false);
  assert.equal((await vetEndpoint('https://relay.example/x', publicLookup)).ok, true);
  assert.equal((await vetEndpoint('https://[64:ff9b::7f00:1]/x', publicLookup)).ok, false);
});

/* ── delivery ── */

class FakeSocket extends EventEmitter {
  constructor(options, answer) {
    super();
    this.options = options;
    this.written = '';
    this.destroyed = false;
    setImmediate(() => {
      this.emit('secureConnect');
      if (answer !== null) this.emit('data', answer);
    });
  }

  write(text) {
    this.written += text;
  }

  destroy() {
    this.destroyed = true;
  }
}

test('delivery connects to the vetted address, names the host for TLS, and reads the status', async () => {
  let socket;
  const vetted = await vetEndpoint('https://relay.example:8443/up?x=1', publicLookup);
  const status = await deliver(vetted, {
    connect: (options) => (socket = new FakeSocket(options, 'HTTP/1.1 410 Gone\r\n\r\n'))
  });

  assert.equal(status, 410);
  assert.equal(socket.options.host, '93.184.216.34', 'pinned to the checked address');
  assert.equal(socket.options.servername, 'relay.example', 'certificate checked against the name');
  assert.equal(socket.options.port, 8443);
  assert.match(socket.written, /^POST \/up\?x=1 HTTP\/1\.1\r\nHost: relay\.example:8443\r\n/);
  assert.match(socket.written, /Content-Length: 0/);
  assert.equal(socket.destroyed, true);
});

test('delivery gives up on a relay that never answers', async () => {
  const vetted = await vetEndpoint('https://relay.example/up', publicLookup);

  await assert.rejects(deliver(vetted, { connect: (options) => new FakeSocket(options, null), timeoutMs: 20 }));
});

/* ── rows ── */

test('concurrent changes to one row are applied one after another, not lost', async () => {
  const ctx = fakeCtx({ 1: { status: 'hi' } });
  const rows = createRows(ctx);

  await Promise.all([
    rows.updateRow(1, (row) => ({ ...row, mutedChannels: [4] })),
    rows.updateRow(1, (row) => ({ ...row, status: 'busy' })),
    rows.updateRow(1, (row) => ({ ...row, pushEndpoints: ['https://a.example/'] }))
  ]);

  assert.deepEqual(ctx.data.get(1), {
    status: 'busy',
    mutedChannels: [4],
    pushEndpoints: ['https://a.example/']
  });
});

test('a row that cannot be read is never overwritten', async () => {
  const ctx = fakeCtx({ 1: { status: 'keep me', mutedChannels: [3] } });
  const rows = createRows(ctx);

  ctx.failReads.add(1);

  await assert.rejects(rows.updateRow(1, (row) => ({ ...row, pushEndpoints: [] })));
  assert.deepEqual(ctx.data.get(1), { status: 'keep me', mutedChannels: [3] });
});

test('the limiter allows a burst and then refuses until the window passes', () => {
  let clock = 0;
  const allow = createLimiter(2, 1000, () => clock);

  assert.equal(allow('a'), true);
  assert.equal(allow('a'), true);
  assert.equal(allow('a'), false);
  assert.equal(allow('b'), true, 'per key');
  clock = 1001;
  assert.equal(allow('a'), true);
});

/* ── push ── */

const subscribed = () =>
  fakeCtx({
    1: { pushEndpoints: ['https://relay.example/one'], mutedChannels: [9] },
    2: { pushEndpoints: ['https://relay.example/two'] }
  });

const pushFor = async (ctx, options = {}) => {
  const rows = createRows(ctx);
  const push = createPush(ctx, rows, { lookup: publicLookup, send: async () => 200, ...options });

  await primeFromUserRows(ctx, [push.adopt]);

  return { push, rows };
};

test('two messages arriving together push each user once', async () => {
  const ctx = subscribed();
  const { push } = await pushFor(ctx);

  const [first, second] = await Promise.all([
    push.recipients({ userId: 99, channelId: 4 }),
    push.recipients({ userId: 99, channelId: 4 })
  ]);

  assert.equal(first.length + second.length, 2, 'users 1 and 2, once each');
});

test('mutes, permissions and the author are respected, without reading rows per message', async () => {
  const ctx = subscribed();
  let reads = 0;
  const get = ctx.userData.get;

  const { push } = await pushFor(ctx);

  ctx.userData.get = async (id) => {
    reads += 1;

    return get(id);
  };
  ctx.permissions.userCanInChannel = async (userId) => userId !== 2;

  const wanted = await push.recipients({ userId: 1, channelId: 9 });

  assert.deepEqual(wanted.map((entry) => entry.userId), [], 'author skipped, user 2 cannot see it');
  assert.equal(reads, 0);

  // user 2 was refused, so that message did not use up their debounce window
  ctx.permissions.userCanInChannel = async () => true;
  assert.deepEqual((await push.recipients({ userId: 5, channelId: 4 })).map((entry) => entry.userId).sort(), [1, 2]);
});

test('a dead endpoint is dropped without touching the rest of the row', async () => {
  const ctx = fakeCtx({ 1: { pushEndpoints: ['https://relay.example/one'], status: 'hello', mutedChannels: [2] } });
  const { push } = await pushFor(ctx, { send: async () => 410 });

  await push.onMessage({ userId: 7, channelId: 4 });
  await new Promise((resolve) => setTimeout(resolve, 20));

  assert.deepEqual(ctx.data.get(1), { pushEndpoints: [], status: 'hello', mutedChannels: [2] });
});

test('a refused registration says only that it was refused, and is rate limited', async () => {
  const ctx = fakeCtx({});
  const rows = createRows(ctx);
  const push = createPush(ctx, rows, { lookup: async () => [{ address: '10.0.0.1', family: 4 }] });

  await assert.rejects(push.register(1, 'https://internal.example/'), { message: REFUSED });

  const ok = createPush(ctx, rows, { lookup: publicLookup });

  for (let index = 0; index < 10; index += 1) await ok.register(3, `https://relay.example/${index}`);
  await assert.rejects(ok.register(3, 'https://relay.example/again'), /Too many/);
  assert.equal(ctx.data.get(3).pushEndpoints.length, 5, 'capped');
});

/* ── statuses and settings ── */

test('a status loses invisible and control characters and is capped', () => {
  assert.equal(statusFrom('  hello\n\tthere  '), 'hello there');
  assert.equal(statusFrom('ad\u202emin'), 'admin');
  assert.equal(statusFrom('a\u200bb\ufeffc'), 'abc');
  assert.equal([...statusFrom('😀'.repeat(200))].length, 100);
  assert.equal(statusFrom(42), '');
});

test('setting a status keeps the rest of the row and is rate limited', async () => {
  const ctx = fakeCtx({ 1: { mutedChannels: [5] } });
  const statuses = createStatuses(ctx, createRows(ctx));

  assert.deepEqual(await statuses.set(1, 'out'), { status: 'out' });
  assert.deepEqual(ctx.data.get(1), { mutedChannels: [5], status: 'out' });
  assert.deepEqual(statuses.own(1), { status: 'out' });

  for (let index = 0; index < 9; index += 1) await statuses.set(1, `n${index}`);
  await assert.rejects(statuses.set(1, 'too many'), /Too many/);
});

test('mute lists and floors are validated on the way in', () => {
  assert.deepEqual(mutedFrom([3, 3, -1, 1.5, '4', 7]), [3, 7]);
  assert.deepEqual(floorFrom({ 12: 3, 40: 2.6, x: 1, 5: -1, '07': 1 }), { 12: 3, 40: 3 });
  assert.equal(floorFrom([1, 2]), null);
});

/* ── migration ── */

test('the old settings file is kept when a user could not be carried over', async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'shiver-plugin-'));
  const file = path.join(dir, 'user-settings.json');

  await writeFile(file, JSON.stringify({ 1: { mutedChannels: [4] }, 2: { mutedChannels: [5] } }));

  const ctx = { ...fakeCtx({}), dataPath: dir, path: dir };

  ctx.failReads.add(2);
  await adoptOldStore(ctx, createRows(ctx));

  assert.deepEqual(ctx.data.get(1), { mutedChannels: [4] });
  assert.ok(await readFile(file, 'utf-8'), 'still there for the next load');

  ctx.failReads.clear();
  await adoptOldStore(ctx, createRows(ctx));
  await assert.rejects(readFile(file, 'utf-8'), 'deleted once everything moved');
});

/* ── files ── */

test('stored names always get a random suffix and stay within filesystem limits', () => {
  assert.equal(uniqueName('photo.png', 'abc'), 'photo~abc.png');
  assert.equal(uniqueName('.env', 'abc'), '.env~abc');
  assert.deepEqual(splitName('a.tar.gz'), { base: 'a.tar', ext: '.gz' });
  assert.ok(Buffer.byteLength(uniqueName(`${'é'.repeat(300)}.png`)) <= 255);
});
