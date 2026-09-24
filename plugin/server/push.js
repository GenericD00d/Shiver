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

import { lookup as dnsLookup } from 'node:dns/promises';
import { isIP } from 'node:net';
import { connect as tlsConnect } from 'node:tls';

import { createLimiter } from './rows.js';

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

/* ── addresses ── */

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

/* ── endpoints ── */

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

/* ── the feature ── */

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
