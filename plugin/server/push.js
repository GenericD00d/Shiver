/**
 * Waking a phone that is not running Shiver.
 *
 * Shiver's mobile client notifies from its own background sockets, which means it notifies only while
 * its process is alive. Android eventually kills it, and from then on the phone is silent. The usual
 * answer is Google FCM, which would make a self-hosted client depend on Google, so Shiver uses
 * **UnifiedPush** instead: the phone runs a distributor (ntfy, self-hosted or not), Shiver registers
 * with it and gets back an endpoint URL, and this file posts to that URL when something arrives.
 *
 * Three properties are deliberate.
 *
 * **The push carries nothing.** Not the message, not the author, not the channel — an empty body.
 * The relay sits outside this server, so anything in the payload is something a third party gets to
 * read. Shiver already knows how to connect and work out what it missed; all it needs from us is
 * "look again". A leaked endpoint therefore buys an attacker the ability to make a phone check its
 * messages, not the messages.
 *
 * **One endpoint per server.** Shiver registers a separate UnifiedPush instance per rail entry, so the
 * endpoint that receives the ping is itself the identifier — this file never has to say which server
 * it is, and Shiver never has to be told something it already knew.
 *
 * **The user's own rules are applied here**, not on the phone: their Shiver mutes (the same row this
 * plugin already keeps) and Sharkord's own channel permissions. A push that a muted channel caused,
 * or that names a channel the user cannot read, should not be sent at all rather than sent and
 * discarded — the phone cannot un-wake itself.
 *
 * **What is deliberately *not* decided here is whether the user is online.** It was, until 0.4.4:
 * `user:joined` and `user:left` fed a set, and anyone in it was skipped on the grounds that Shiver was
 * already running and had its own socket. Two things were wrong with that. Sharkord emits
 * `user:left` only when a user's *last* socket closes (`handleSocketClose`), so the set means "this
 * account has a connection open somewhere" — and Shiver holds one per server from every device it
 * runs on, which is how the inbox works at all. So a desktop Shiver left open made every push to that
 * user's phone be skipped, and the feature could not fire for anyone who used Shiver on more than one
 * device. It had never once fired.
 *
 * The server cannot tell *which* device is awake, and does not need to: the phone already knows.
 * `PushReceiver` stays silent when Shiver is running in that process, and the running app turns a push
 * into an inbox sync rather than a notification. Suppression belongs where the knowledge is, and the
 * cost of being wrong here is one empty POST that the phone ignores.
 */

import { lookup } from 'node:dns/promises';
import { isIP } from 'node:net';

/** How many endpoints one user may register. Enough for a phone and a tablet, not a list. */
const MAX_ENDPOINTS = 5;

/** Longest an endpoint URL may be, so a row cannot be filled with one. */
const MAX_ENDPOINT_LENGTH = 512;

/** At most one push per user per this long, however much arrives. */
const DEBOUNCE_MS = 10_000;

/** A push is a hint, not a delivery: if the relay is slow, give up rather than pile up. */
const PUSH_TIMEOUT_MS = 5_000;

/**
 * Endpoints that must never be posted to.
 *
 * The URL comes from a user, and this server is the one that would fetch it — so without this a
 * user could point the endpoint at `http://localhost:6379` and use the plugin to reach services
 * on the server's own network that they cannot reach themselves. That is server-side request
 * forgery, and the fact that the body is empty does not make it harmless: the timing and the error
 * tell an attacker what is listening.
 *
 * https only, matching the rule Shiver applies to everything else it talks to.
 */
const isPrivateAddress = (address) => {
  if (isIP(address) === 6) {
    const plain = address.toLowerCase().replace(/^\[|\]$/g, '');

    return (
      plain === '::1' ||
      plain === '::' ||
      plain.startsWith('fc') ||
      plain.startsWith('fd') ||
      plain.startsWith('fe80') ||
      plain.startsWith('::ffff:')
    );
  }

  const parts = address.split('.').map(Number);

  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part))) return true;

  const [a, b] = parts;

  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 100 && b >= 64 && b <= 127) ||
    a >= 224
  );
};

/**
 * Whether an endpoint is one this server is willing to post to.
 *
 * Resolved rather than pattern-matched: a hostname can point anywhere, so the name being public
 * says nothing about where it lands. Both families are checked, because a host that answers with a
 * public A record and a loopback AAAA would otherwise walk straight through.
 */
const endpointAllowed = async (endpoint) => {
  let url;

  try {
    url = new URL(endpoint);
  } catch {
    return { ok: false, why: 'not a url' };
  }

  if (url.protocol !== 'https:') return { ok: false, why: 'not https' };
  if (endpoint.length > MAX_ENDPOINT_LENGTH) return { ok: false, why: 'too long' };

  const host = url.hostname.replace(/^\[|\]$/g, '');

  if (isIP(host)) {
    return isPrivateAddress(host) ? { ok: false, why: 'private address' } : { ok: true };
  }

  let addresses;

  try {
    addresses = await lookup(host, { all: true });
  } catch {
    return { ok: false, why: 'does not resolve' };
  }

  if (!addresses.length) return { ok: false, why: 'does not resolve' };
  if (addresses.some((entry) => isPrivateAddress(entry.address))) {
    return { ok: false, why: 'resolves to a private address' };
  }

  return { ok: true };
};

/** A stored value read back as an endpoint list, keeping only what an endpoint can be. */
export const endpointsFrom = (value) =>
  Array.isArray(value)
    ? [
        ...new Set(
          value.filter(
            (entry) =>
              typeof entry === 'string' &&
              entry.length <= MAX_ENDPOINT_LENGTH &&
              entry.startsWith('https://')
          )
        )
      ].slice(0, MAX_ENDPOINTS)
    : [];

/**
 * Everything this file needs to remember, which is deliberately not much.
 *
 * `subscribers` exists so a message does not cost a read of every user's row. It is built once at
 * load and kept in step as people register, so the per-message work is proportional to the number of
 * people who actually want waking rather than to the size of the server.
 */
export const createPush = (ctx) => {
  const lastPushAt = new Map();
  const subscribers = new Set();

  const muted = (stored, channelId) =>
    Array.isArray(stored?.mutedChannels) && stored.mutedChannels.includes(channelId);

  /**
   * Posts the wake-up, and forgets an endpoint the relay says is gone.
   *
   * `410 Gone` and `404` are the distributor telling us this registration no longer exists — a
   * phone that uninstalled Shiver or changed distributor. Left alone it would be posted to forever,
   * so it is dropped from the row on the spot.
   */
  const wake = async (userId, endpoint) => {
    const control = new AbortController();
    const timer = setTimeout(() => control.abort(), PUSH_TIMEOUT_MS);

    try {
      const response = await fetch(endpoint, {
        method: 'POST',
        signal: control.signal,
        // an empty body on purpose: see the note at the top of this file
        headers: { 'Content-Length': '0', 'TTL': '2419200' }
      });

      if (response.status === 404 || response.status === 410) {
        await forget(userId, endpoint);
        ctx.logger.debug(`Dropped a dead push endpoint for user ${userId}`);
      }
    } catch (error) {
      // a relay that is down is not this server's problem to solve, and not worth a loud log:
      // the phone will catch up the next time Shiver runs
      ctx.logger.debug(`Could not reach a push endpoint for user ${userId}: ${error?.message}`);
    } finally {
      clearTimeout(timer);
    }
  };

  const forget = async (userId, endpoint) => {
    const stored = await ctx.userData.get(userId).catch(() => null);
    const endpoints = endpointsFrom(stored?.pushEndpoints).filter((entry) => entry !== endpoint);

    await ctx.userData.set(userId, { ...stored, pushEndpoints: endpoints }).catch(() => {});

    if (!endpoints.length) subscribers.delete(userId);
  };

  /** Who should be woken for a message, and why each of the others should not. */
  const recipients = async (message) => {
    const wanted = [];

    for (const userId of subscribers) {
      // their own message, arriving back at them
      if (userId === message.userId) continue;

      const since = lastPushAt.get(userId) ?? 0;

      if (Date.now() - since < DEBOUNCE_MS) continue;

      const stored = await ctx.userData.get(userId).catch(() => null);
      const endpoints = endpointsFrom(stored?.pushEndpoints);

      if (!endpoints.length) {
        subscribers.delete(userId);
        continue;
      }

      // the user's own mute, kept by this same plugin, applied before the phone is disturbed
      if (muted(stored, message.channelId)) continue;

      const allowed = await ctx.permissions
        .userCanInChannel(userId, message.channelId, 'VIEW_CHANNEL')
        .catch(() => false);

      if (!allowed) continue;

      wanted.push({ userId, endpoints });
    }

    return wanted;
  };

  const onMessage = async (message) => {
    if (!subscribers.size) return;

    let wanted;

    try {
      wanted = await recipients(message);
    } catch (error) {
      ctx.logger.debug(`Could not work out who to wake: ${error?.message}`);

      return;
    }

    for (const { userId, endpoints } of wanted) {
      lastPushAt.set(userId, Date.now());

      // not awaited together with the loop: one slow relay must not hold up the others, and none of
      // them may hold up the message that caused this
      for (const endpoint of endpoints) void wake(userId, endpoint);
    }
  };

  /**
   * Records an endpoint for the calling user.
   *
   * Called through the client relay, so `userId` is whoever is signed in on that page — there is no
   * path here for one user to register an endpoint against another's row.
   */
  const register = async (userId, endpoint) => {
    const allowed = await endpointAllowed(endpoint);

    if (!allowed.ok) {
      ctx.logger.debug(`Refused a push endpoint for user ${userId}: ${allowed.why}`);

      throw new Error(`That push endpoint was refused: ${allowed.why}`);
    }

    const stored = await ctx.userData.get(userId);
    const endpoints = endpointsFrom(stored?.pushEndpoints);

    if (!endpoints.includes(endpoint)) {
      endpoints.unshift(endpoint);
      await ctx.userData.set(userId, {
        ...stored,
        pushEndpoints: endpoints.slice(0, MAX_ENDPOINTS)
      });
    }

    subscribers.add(userId);

    return endpointsFrom(endpoints);
  };

  const unregister = async (userId, endpoint) => {
    const stored = await ctx.userData.get(userId);
    const endpoints = endpointsFrom(stored?.pushEndpoints).filter(
      (entry) => endpoint === undefined || entry !== endpoint
    );

    await ctx.userData.set(userId, { ...stored, pushEndpoints: endpoints });

    if (!endpoints.length) subscribers.delete(userId);

    return endpoints;
  };

  /**
   * Learns who already has an endpoint, once, at load.
   *
   * Costs one row read per user on a server that has never used push, which is the price of not
   * having a "who has data" query. It happens at startup rather than per message, and a server with
   * no subscribers ends up doing no per-message work at all.
   */
  const start = async () => {
    let users;

    try {
      users = await ctx.users.list();
    } catch (error) {
      ctx.logger.debug(`Could not list users to find push subscribers: ${error?.message}`);

      return;
    }

    for (const user of users) {
      const stored = await ctx.userData.get(user.id).catch(() => null);

      if (endpointsFrom(stored?.pushEndpoints).length) subscribers.add(user.id);
    }

    if (subscribers.size) {
      ctx.logger.log(`Shiver push: ${subscribers.size} user(s) can be woken when they are away`);
    }
  };

  return {
    start,
    register,
    unregister,
    onMessage,
    // for the relay to report state without reaching into any of the above
    isSubscribed: (userId) => subscribers.has(userId)
  };
};
