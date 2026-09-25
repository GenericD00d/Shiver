/**
 * Shiver companion plugin, server half: registers the actions the client relay calls, migrates the
 * settings file older versions kept, and wires message events to push.
 *
 * Every action acts on `invoker.userId` from the session, never an id in the payload, and every
 * write is rate limited per user.
 *
 * One file on purpose: a server that loads only this entry afresh cannot run it against older
 * copies of any other (which failed as `take is not a function` after an update).
 */

import { Buffer } from 'node:buffer';
import { randomBytes } from 'node:crypto';
import { lookup as dnsLookup } from 'node:dns/promises';
import { readFile, unlink } from 'node:fs/promises';
import { isIP } from 'node:net';
import path from 'node:path';
import { connect as tlsConnect } from 'node:tls';

/* ── rows ── */

/**
 * Serialised access to each user's plugin row.
 *
 * The host only offers "read row" and "replace row", and every feature stores its field in the same
 * row, so all changes go through `updateRow`, which runs one change per user at a time. A row that
 * cannot be read is never overwritten.
 */
export const createRows = (ctx) => {
  const queues = new Map();

  /** Runs `change(copyOfRow)` and stores its result; `undefined` leaves the row as it was. */
  const updateRow = (userId, change) => {
    const next = (queues.get(userId) ?? Promise.resolve())
      .catch(() => undefined)
      .then(async () => {
        const stored = (await ctx.userData.get(userId)) ?? {};
        const updated = await change({ ...stored });

        if (updated === undefined) return stored;

        await ctx.userData.set(userId, updated);

        return updated;
      });

    queues.set(userId, next);

    const cleanup = () => {
      if (queues.get(userId) === next) queues.delete(userId);
    };

    next.then(cleanup, cleanup);

    return next;
  };

  /** Reads a row after any queued change to it has landed. */
  const readRow = async (userId) => {
    await (queues.get(userId) ?? Promise.resolve()).catch(() => undefined);

    return (await ctx.userData.get(userId)) ?? {};
  };

  return { updateRow, readRow };
};

/** Returns `allow(key)`: true at most `limit` times per `windowMs` for each key. */
export const createLimiter = (limit, windowMs, now = () => Date.now()) => {
  const seen = new Map();

  return (key) => {
    const at = now();
    const recent = (seen.get(key) ?? []).filter((time) => at - time < windowMs);
    const allowed = recent.length < limit;

    if (allowed) recent.push(at);

    seen.set(key, recent);

    return allowed;
  };
};

/* ── file names ── */

/**
 * Gives every stored file an unpredictable name.
 *
 * Sharkord reuses a filename once the old file is deleted, and clients cache `/public/<name>` for an
 * hour, so a new upload could show up as someone's old, deleted picture. A random suffix prevents it.
 */


/** 64 bits: a birthday collision on one base name is out of reach. */
const SUFFIX_BYTES = 8;
const SUFFIX_MARK = '~';
/** Keeps name + suffix + extension under the usual 255-byte filesystem limit. */
const MAX_BASE_BYTES = 180;

/** Splits at the last dot, so the suffix lands before the extension. */
export const splitName = (name) => {
  const at = name.lastIndexOf('.');

  return at > 0 ? { base: name.slice(0, at), ext: name.slice(at) } : { base: name, ext: '' };
};

/** Cuts text to at most `limit` UTF-8 bytes without splitting a character. */
const clampBytes = (text, limit) => {
  let out = '';
  let bytes = 0;

  for (const character of text) {
    bytes += Buffer.byteLength(character);

    if (bytes > limit) break;

    out += character;
  }

  return out;
};

/**
 * `photo.png` -> `photo~<random>.png`. Always appends; the hook never sees its own output. Path
 * separators and control characters become `_`, whatever the host does with the name afterwards.
 */
export const uniqueName = (raw, random = randomBytes(SUFFIX_BYTES).toString('hex')) => {
  const name = raw.replace(/[/\\\x00-\x1f\x7f]/g, '_');
  const { base, ext } = splitName(name);
  const [stem, tail] = Buffer.byteLength(ext) > 32 ? [name, ''] : [base, ext];

  return `${clampBytes(stem, MAX_BASE_BYTES)}${SUFFIX_MARK}${random}${tail}`;
};

/** Installs the rename hook and returns the host's unsubscribe, for unload. */
const installFileNaming = (ctx) =>
  ctx.hooks.onBeforeFileSave(async ({ originalName, type }) => {
    const renamed = uniqueName(String(originalName ?? 'file'));

    ctx.logger.debug(`Shiver: storing a ${type} as ${renamed}`);

    return { update: { originalName: renamed } };
  });

/* ── settings ── */

/** The muted-channel list and shared unread floor, validated and written through `rows`. */

const MAX_MUTED_CHANNELS = 500;
const MAX_FLOOR_CHANNELS = 5000;

/** Positive integer channel ids, de-duplicated and capped. */
export const mutedFrom = (value) =>
  Array.isArray(value)
    ? [...new Set(value.filter((id) => Number.isInteger(id) && id > 0))].slice(0, MAX_MUTED_CHANNELS)
    : [];

/** `{ "<channel id>": count }` with valid keys and non-negative counts (rounded), or null. */
export const floorFrom = (value) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;

  const floor = {};
  let kept = 0;

  for (const [key, count] of Object.entries(value)) {
    if (kept >= MAX_FLOOR_CHANNELS) break;
    if (!/^[1-9]\d{0,15}$/.test(key) || typeof count !== 'number' || !Number.isFinite(count) || count < 0) {
      continue;
    }

    floor[key] = Math.min(Math.round(count), 0xffffffff);
    kept += 1;
  }

  return floor;
};

const createSettings = (rows, onChange = () => undefined) => ({
  getMutedChannels: async (userId) => ({ mutedChannels: mutedFrom((await rows.readRow(userId)).mutedChannels) }),

  setMutedChannels: async (userId, value) => {
    const mutedChannels = mutedFrom(value);

    onChange(userId, await rows.updateRow(userId, (row) => ({ ...row, mutedChannels })));

    return { mutedChannels };
  },

  setReadFloor: async (userId, value) => {
    const readFloor = floorFrom(value);

    if (!readFloor) throw new Error('That is not an unread floor');

    await rows.updateRow(userId, (row) => ({ ...row, readFloor }));

    return { ok: true };
  }
});

/* ── statuses ── */

/**
 * Custom statuses: a line each user writes about themselves, shown beside their name via
 * Sharkord's plugin slots and broadcast with `ctx.push.toAll` when it changes.
 */


const MAX_STATUS = 100;
const STATUS_LIMIT = 10;

/** Bidi overrides, zero-width and other invisible format characters: only useful for spoofing. */
const INVISIBLE =
  /[\u00ad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180b-\u180f\u200b-\u200f\u202a-\u202e\u2060-\u206f\u3164\ufe00-\ufe0f\ufeff\uffa0\u{e0000}-\u{e0fff}]/gu;

/** A value as a status: visible characters only, one line, at most MAX_STATUS characters. */
export const statusFrom = (value) =>
  typeof value === 'string'
    ? [
        ...value
          .replace(INVISIBLE, '')
          // eslint-disable-next-line no-control-regex
          .replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ')
          .replace(/\s+/g, ' ')
          .trim()
      ]
        .slice(0, MAX_STATUS)
        .join('')
        .trim()
    : '';

export const createStatuses = (ctx, rows, { now = () => Date.now() } = {}) => {
  /** userId -> status, filled at load and kept current by writes. */
  const statuses = new Map();
  const mayChange = createLimiter(STATUS_LIMIT, 60_000, now);

  const adopt = (userId, row) => {
    const status = statusFrom(row?.status);

    if (status) statuses.set(userId, status);
  };

  adopt.done = () => {
    if (statuses.size) ctx.logger.log(`Shiver: ${statuses.size} user(s) have a status set`);
  };

  /** Sets (or, with '', clears) the caller's status and tells everyone. Rate limited. */
  const set = async (userId, value) => {
    if (!mayChange(userId)) throw new Error('Too many status changes; try again in a minute');

    const status = statusFrom(value);

    await rows.updateRow(userId, (row) => ({ ...row, status }));

    if (status) statuses.set(userId, status);
    else statuses.delete(userId);

    ctx.push.toAll({ kind: 'status', userId, status });

    return { status };
  };

  const all = () => ({ statuses: [...statuses].map(([userId, status]) => ({ userId, status })) });

  const own = (userId) => ({ status: statuses.get(userId) ?? '' });

  const forget = (userId) => {
    if (statuses.delete(userId)) ctx.push.toAll({ kind: 'status', userId, status: '' });
  };

  return { adopt, set, all, own, forget };
};

/* ── push ── */

/**
 * Wakes a phone that is not running Shiver, over UnifiedPush.
 *
 * On each message, every subscribed user who did not write it, has not muted the channel and can
 * see it gets an empty POST to their registered endpoints (at most once per DEBOUNCE_MS). The body is
 * empty because the relay is a third party; the phone reconnects to find out what arrived.
 *
 * Endpoints are user-supplied URLs this server fetches, so they are vetted (https on 443, every
 * resolved address public) and the request is made over TLS to the exact address that was vetted,
 * which closes DNS rebinding. Redirects are never followed, and deliveries in flight are capped.
 */



const MAX_ENDPOINTS = 5;
const MAX_ENDPOINT_LENGTH = 512;
const DEBOUNCE_MS = 10_000;
const PUSH_TIMEOUT_MS = 5_000;
const MAX_RESPONSE_BYTES = 16 * 1024;
const MAX_IN_FLIGHT = 64;
/** Registrations per user per minute; each costs a DNS lookup. */
const REGISTER_LIMIT = 10;

/** The only reason a refused registration is given, so the check cannot probe internal DNS. */
export const REFUSED = 'That push endpoint was refused';

/* ── push: addresses ── */

/** The eight 16-bit groups of an IPv6 address, or null. */
const ipv6Groups = (address) => {
  if ((address.match(/::/g) ?? []).length > 1) return null;

  let text = address;
  const quad = text.match(/(\d+\.\d+\.\d+\.\d+)$/);

  if (quad) {
    const parts = quad[1].split('.').map(Number);

    if (parts.some((part) => !Number.isInteger(part) || part > 255)) return null;

    text =
      text.slice(0, -quad[1].length) +
      `${((parts[0] << 8) | parts[1]).toString(16)}:${((parts[2] << 8) | parts[3]).toString(16)}`;
  }

  const hasGap = text.includes('::');
  const [head, tail] = text.split('::');
  const parse = (part) =>
    part ? part.split(':').map((group) => (/^[0-9a-f]{1,4}$/i.test(group) ? parseInt(group, 16) : NaN)) : [];
  const left = parse(head);
  const right = hasGap ? parse(tail) : [];
  const missing = 8 - left.length - right.length;

  if (missing < 0 || (!hasGap && missing !== 0)) return null;

  const groups = [...left, ...Array(missing).fill(0), ...right];

  return groups.every(Number.isInteger) ? groups : null;
};

const isPrivateV4 = (address) => {
  const parts = address.split('.').map(Number);

  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) {
    return true;
  }

  const [a, b, c] = parts;

  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 100 && b >= 64 && b <= 127) ||
    (a === 192 && b === 0 && (c === 0 || c === 2)) ||
    (a === 192 && b === 88 && c === 99) ||
    (a === 198 && b === 51 && c === 100) ||
    (a === 203 && b === 0 && c === 113) ||
    (a === 198 && (b === 18 || b === 19)) ||
    a >= 224
  );
};

/**
 * Allow-list: only global unicast (2000::/3) passes. Inside it, 6to4 is unwrapped and checked as
 * IPv4, and Teredo, documentation and ORCHID ranges are refused. Everything else — loopback,
 * IPv4-mapped/translated, NAT64, ULA, link/site-local, multicast — fails the first test.
 */
const isPrivateV6 = (address) => {
  const groups = ipv6Groups(address);

  if (!groups) return true;

  const [first, second] = groups;

  if ((first & 0xe000) !== 0x2000) return true;

  if (first === 0x2002) {
    const [, high, low] = groups;

    return isPrivateV4(`${high >> 8}.${high & 0xff}.${low >> 8}.${low & 0xff}`);
  }

  return first === 0x2001 && (second === 0 || second === 0xdb8 || (second >= 0x10 && second <= 0x2f));
};

/** Whether this server must refuse to connect to a literal IP address. Non-IPs are refused. */
export const isPrivateAddress = (address) => {
  const plain = String(address).toLowerCase().replace(/^\[|\]$/g, '');
  const family = isIP(plain);

  if (family === 6) return isPrivateV6(plain);
  if (family === 4) return isPrivateV4(plain);

  return true;
};

/* ── push: endpoints ── */

/** The canonical https URL for an endpoint, or null. Stored and compared in this form only. */
export const normaliseEndpoint = (value) => {
  if (typeof value !== 'string' || value.length > MAX_ENDPOINT_LENGTH) return null;

  let url;

  try {
    url = new URL(value.trim());
  } catch {
    return null;
  }

  if (url.protocol !== 'https:' || url.port || url.username || url.password || !url.hostname) return null;

  url.hash = '';

  return url.href.length <= MAX_ENDPOINT_LENGTH ? url.href : null;
};

/** A stored value read as a de-duplicated, capped endpoint list. */
export const endpointsFrom = (value) =>
  Array.isArray(value)
    ? [...new Set(value.map(normaliseEndpoint).filter(Boolean))].slice(0, MAX_ENDPOINTS)
    : [];

/** Resolves an endpoint and returns the address to connect to, or why it is refused. */
export const vetEndpoint = async (endpoint, lookup = dnsLookup) => {
  const canonical = normaliseEndpoint(endpoint);

  if (!canonical) return { ok: false, why: 'not an https url' };

  const url = new URL(canonical);
  const host = url.hostname.replace(/^\[|\]$/g, '');

  if (isIP(host)) {
    return isPrivateAddress(host) ? { ok: false, why: 'private address' } : { ok: true, url, address: host };
  }

  let addresses;

  try {
    addresses = await lookup(host, { all: true, verbatim: true });
  } catch {
    return { ok: false, why: 'does not resolve' };
  }

  if (!addresses?.length) return { ok: false, why: 'does not resolve' };
  if (addresses.some((entry) => isPrivateAddress(entry.address))) {
    return { ok: false, why: 'resolves to a private address' };
  }

  return { ok: true, url, address: addresses[0].address };
};

/**
 * POSTs an empty body to the vetted address over TLS (certificate checked against the hostname via
 * SNI) and resolves with the HTTP status code. Nothing is followed and only the status line is read.
 */
export const deliver = ({ url, address }, { connect = tlsConnect, timeoutMs = PUSH_TIMEOUT_MS } = {}) =>
  new Promise((resolve, reject) => {
    const hostname = url.hostname.replace(/^\[|\]$/g, '');
    const socket = connect({
      host: address,
      port: 443,
      servername: isIP(hostname) ? undefined : hostname,
      ALPNProtocols: ['http/1.1']
    });

    let received = '';
    let settled = false;

    const finish = (error, status) => {
      if (settled) return;

      settled = true;
      clearTimeout(timer);
      socket.destroy();

      if (error) reject(error);
      else resolve(status);
    };

    const timer = setTimeout(() => finish(new Error('the relay did not answer in time')), timeoutMs);

    socket.setEncoding?.('latin1');
    socket.on('secureConnect', () =>
      socket.write(
        `POST ${url.pathname}${url.search} HTTP/1.1\r\nHost: ${url.host}\r\nContent-Length: 0\r\n` +
          'TTL: 86400\r\nConnection: close\r\nUser-Agent: shiver-plugin\r\n\r\n'
      )
    );
    socket.on('data', (chunk) => {
      received += chunk;

      const match = received.match(/^HTTP\/1\.[01] (\d{3})/);

      if (match) finish(null, Number(match[1]));
      else if (received.includes('\r\n') || received.length > MAX_RESPONSE_BYTES) {
        finish(new Error('the relay did not answer with http'));
      }
    });
    socket.on('error', (error) => finish(error));
    socket.on('end', () => finish(new Error('the relay closed the connection')));
  });

/* ── push: waking subscribers ── */

/**
 * The push feature. `subscribers` caches each subscribed user's endpoints and mutes, so a message
 * costs no row reads; it stays current because every write to those fields goes through this plugin.
 */
export const createPush = (
  ctx,
  rows,
  { lookup = dnsLookup, send = deliver, now = () => Date.now(), maxInFlight = MAX_IN_FLIGHT } = {}
) => {
  /** userId -> { endpoints, muted: Set } */
  const subscribers = new Map();
  const lastPushAt = new Map();
  const mayRegister = createLimiter(REGISTER_LIMIT, 60_000, now);
  let inFlight = 0;

  const remember = (userId, row) => {
    const endpoints = endpointsFrom(row?.pushEndpoints);

    if (endpoints.length) {
      subscribers.set(userId, {
        endpoints,
        muted: new Set(Array.isArray(row?.mutedChannels) ? row.mutedChannels : [])
      });
    } else {
      subscribers.delete(userId);
      lastPushAt.delete(userId);
    }
  };

  const changeEndpoints = (userId, change) =>
    rows.updateRow(userId, (row) => {
      const updated = { ...row, pushEndpoints: change(endpointsFrom(row.pushEndpoints)) };

      remember(userId, updated);

      return updated;
    });

  /** Re-vets and posts one wake-up; drops the endpoint if the relay says it is gone (404/410). */
  const wake = async (userId, endpoint) => {
    if (inFlight >= maxInFlight) return;

    inFlight += 1;

    try {
      const vetted = await vetEndpoint(endpoint, lookup);

      if (!vetted.ok) return ctx.logger.debug(`Refusing to post to a push endpoint for user ${userId}: ${vetted.why}`);

      const status = await send(vetted);

      if (status === 404 || status === 410) {
        await changeEndpoints(userId, (endpoints) => endpoints.filter((entry) => entry !== endpoint));
      }
    } catch (error) {
      ctx.logger.debug(`Could not wake user ${userId}: ${error?.message}`);
    } finally {
      inFlight -= 1;
    }
  };

  /**
   * Users to wake for a message. The debounce slot is claimed synchronously so concurrent messages
   * cannot both pass; claims refused by the permission check are handed back.
   */
  const recipients = async (message) => {
    const at = now();
    const claimed = [];

    for (const [userId, { endpoints, muted }] of subscribers) {
      if (userId === message.userId || muted.has(message.channelId)) continue;
      if (at - (lastPushAt.get(userId) ?? 0) < DEBOUNCE_MS) continue;

      claimed.push({ userId, endpoints, previous: lastPushAt.get(userId) });
      lastPushAt.set(userId, at);
    }

    const allowed = await Promise.all(
      claimed.map(({ userId }) =>
        ctx.permissions.userCanInChannel(userId, message.channelId, 'VIEW_CHANNEL').catch(() => false)
      )
    );

    return claimed.filter((claim, index) => {
      if (allowed[index]) return true;

      if (lastPushAt.get(claim.userId) === at) {
        if (claim.previous === undefined) lastPushAt.delete(claim.userId);
        else lastPushAt.set(claim.userId, claim.previous);
      }

      return false;
    });
  };

  const onMessage = async (message) => {
    if (!subscribers.size) return;

    try {
      for (const { userId, endpoints } of await recipients(message)) {
        for (const endpoint of endpoints) void wake(userId, endpoint);
      }
    } catch (error) {
      ctx.logger.debug(`Could not work out who to wake: ${error?.message}`);
    }
  };

  /** Vets and stores an endpoint for the caller (newest first, capped). Rate limited. */
  const register = async (userId, endpoint) => {
    if (!mayRegister(userId)) throw new Error('Too many push registrations; try again in a minute');

    const vetted = await vetEndpoint(endpoint, lookup);

    if (!vetted.ok) {
      ctx.logger.debug(`Refused a push endpoint for user ${userId}: ${vetted.why}`);

      throw new Error(REFUSED);
    }

    const canonical = vetted.url.href;
    const row = await changeEndpoints(userId, (endpoints) =>
      [canonical, ...endpoints.filter((entry) => entry !== canonical)].slice(0, MAX_ENDPOINTS)
    );

    return row.pushEndpoints;
  };

  /** Drops one endpoint, or all of them when none is named. */
  const unregister = async (userId, endpoint) => {
    const target = endpoint === undefined ? undefined : normaliseEndpoint(endpoint);
    const row = await changeEndpoints(userId, (endpoints) =>
      endpoint === undefined ? [] : endpoints.filter((entry) => entry !== target)
    );

    return row.pushEndpoints;
  };

  const adopt = (userId, row) => remember(userId, row);

  adopt.done = () => {
    if (subscribers.size) ctx.logger.log(`Shiver push: ${subscribers.size} user(s) can be woken`);
  };

  /** Keeps cached mutes current when they change through this plugin. */
  const rowChanged = (userId, row) => {
    if (subscribers.has(userId)) remember(userId, row);
  };

  const forgetUser = (userId) => {
    subscribers.delete(userId);
    lastPushAt.delete(userId);
  };

  return { adopt, register, unregister, onMessage, rowChanged, forgetUser, recipients };
};

/* ── loading ── */

const STORE_FILE = 'user-settings.json';
const PRIME_CONCURRENCY = 16;
/** Writes a minute per user through the actions without a limit of their own. */
const WRITE_LIMIT = 30;

/**
 * Carries mutes from the pre-0.0.25 settings file into the host's storage, without overwriting a
 * user who already has some. The file is deleted only once every entry has been carried over.
 */
export const adoptOldStore = async (ctx, rows) => {
  const places = new Set([path.join(ctx.dataPath, STORE_FILE), path.join(ctx.path, STORE_FILE)]);

  for (const place of places) {
    let entries;

    try {
      const parsed = JSON.parse(await readFile(place, 'utf-8'));

      if (!parsed || typeof parsed !== 'object') continue;

      entries = Object.entries(parsed);
    } catch {
      continue;
    }

    let moved = 0;
    let failed = 0;

    for (const [id, entry] of entries) {
      const userId = Number(id);
      const mutedChannels = mutedFrom(entry?.mutedChannels);

      if (!Number.isInteger(userId) || !mutedChannels.length) continue;

      try {
        await rows.updateRow(userId, (row) =>
          mutedFrom(row.mutedChannels).length ? undefined : { ...row, mutedChannels }
        );
        moved += 1;
      } catch (error) {
        failed += 1;
        ctx.logger.debug(`Could not carry over Shiver settings for user ${userId}: ${error?.message}`);
      }
    }

    if (failed) {
      ctx.logger.log(`Kept ${place}: ${failed} user(s) could not be carried over yet`);
    } else {
      await unlink(place).catch(() => {});
      ctx.logger.log(`Moved Shiver settings for ${moved} user(s) into Sharkord's own storage`);
    }
  }
};

/** Reads every user's row once (a few at a time) and offers it to each taker. Unreadable rows are skipped. */
export const primeFromUserRows = async (ctx, takers) => {
  let users;

  try {
    users = await ctx.users.list();
  } catch (error) {
    ctx.logger.debug(`Could not list users: ${error?.message}`);

    return;
  }

  let next = 0;

  const worker = async () => {
    while (next < users.length) {
      const { id } = users[next++];
      const row = await ctx.userData.get(id).catch(() => undefined);

      if (row !== undefined) for (const take of takers) take(id, row);
    }
  };

  await Promise.all(Array.from({ length: Math.min(PRIME_CONCURRENCY, users.length) }, worker));

  for (const take of takers) take.done?.();
};

/** Unsubscribers for the current load. */
let stop = [];

const onLoad = async (ctx) => {
  await onUnload(ctx, { quiet: true });

  const rows = createRows(ctx);

  await adoptOldStore(ctx, rows);

  // lists the client bundle, which is the relay Shiver's bridge talks to
  ctx.ui.enable();

  const push = createPush(ctx, rows);
  const statuses = createStatuses(ctx, rows);
  const settings = createSettings(rows, push.rowChanged);

  await primeFromUserRows(ctx, [push.adopt, statuses.adopt]);

  const mayWrite = createLimiter(WRITE_LIMIT, 60_000);
  const limited = (run) => (user, payload) => {
    if (!mayWrite(user)) throw new Error('Too many changes; try again in a minute');

    return run(user, payload);
  };

  const actions = {
    setStatus: ['Set your own status line', (user, payload) => statuses.set(user, payload?.status)],
    getStatuses: ['Everyone who has a status set', () => statuses.all()],
    getOwnStatus: ['Your own status line', (user) => statuses.own(user)],
    getMutedChannels: ['Your muted channels', (user) => settings.getMutedChannels(user)],
    setMutedChannels: [
      'Replace your muted channels',
      limited((user, payload) => settings.setMutedChannels(user, payload?.mutedChannels))
    ],
    setReadFloor: [
      'Store your shared unread floor',
      limited((user, payload) => settings.setReadFloor(user, payload?.floor))
    ],
    setPushEndpoint: [
      'Register a UnifiedPush endpoint so Shiver can wake this device',
      async (user, payload) => ({ endpoints: await push.register(user, payload?.endpoint) })
    ],
    clearPushEndpoint: [
      'Stop waking a device, or all of them when no endpoint is named',
      limited(async (user, payload) => ({
        endpoints: await push.unregister(user, typeof payload?.endpoint === 'string' ? payload.endpoint : undefined)
      }))
    ]
  };

  for (const [name, [description, run]] of Object.entries(actions)) {
    ctx.actions.register({ name, description, executes: async (invoker, payload) => run(invoker.userId, payload) });
  }

  stop = [
    installFileNaming(ctx),
    ctx.events.on('message:created', (message) => void push.onMessage(message)),
    ctx.events.on('user:deleted', ({ userId }) => {
      statuses.forget(userId);
      push.forgetUser(userId);
    })
  ];

  ctx.logger.log('Shiver plugin loaded');
};

const onUnload = async (ctx, { quiet = false } = {}) => {
  for (const off of stop) {
    try {
      if (typeof off === 'function') off();
    } catch {
      // an unsubscribe that throws is not worth failing an unload over
    }
  }

  stop = [];

  if (!quiet) ctx.logger.log('Shiver plugin unloaded');
};

export { onLoad, onUnload };
