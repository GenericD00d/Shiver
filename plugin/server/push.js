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

/** The 16-bit groups of an IPv6 address, expanding the one `::` run. */
const ipv6Groups = (address) => {
  const [head, tail] = address.split('::');
  const parse = (part) => (part ? part.split(':').map((group) => parseInt(group, 16)) : []);
  const left = parse(head);
  const right = address.includes('::') ? parse(tail) : [];
  const groups = [...left, ...Array(8 - left.length - right.length).fill(0), ...right];

  return groups.length === 8 && groups.every(Number.isInteger) ? groups : null;
};

/** The IPv4 inside an IPv4-mapped or IPv4-compatible address, as dotted quad. */
const embeddedV4 = (groups) => {
  const leading = groups.slice(0, 5).every((group) => group === 0);

  if (!leading) return null;

  // ::ffff:a.b.c.d (mapped) and ::a.b.c.d (compatible, deprecated but still routed by some stacks)
  if (groups[5] !== 0xffff && groups[5] !== 0) return null;

  const [, , , , , , high, low] = groups;

  return `${high >> 8}.${high & 0xff}.${low >> 8}.${low & 0xff}`;
};

const isPrivateV6 = (address) => {
  const groups = ipv6Groups(address);

  // an address `isIP` called v6 but that will not expand is not one to take a chance on
  if (!groups) return true;

  // **Checked on the groups, not on the text.** The WHATWG URL parser rewrites an IPv6 host into
  // its compressed form before any of this sees it, so `::ffff:127.0.0.1` arrives as
  // `::ffff:7f00:1` — and a check that matched the dotted spelling, or the old
  // `startsWith('::ffff:')`, is a check that has already been normalised out from under it.
  const embedded = embeddedV4(groups);

  if (embedded !== null) return isPrivateAddress(embedded);

  const [first] = groups;

  return (
    // ::1 loopback and :: unspecified
    groups.slice(0, 7).every((group) => group === 0) ||
    // fc00::/7, unique local
    (first & 0xfe00) === 0xfc00 ||
    // fe80::/10, link local — fe80 through febf, which a `startsWith('fe80')` missed
    (first & 0xffc0) === 0xfe80
  );
};

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
 *
 * **Ranges are matched on the parsed address, not on a string prefix.** `fe80::/10` runs from
 * `fe80` to `febf`, so `startsWith('fe80')` — which is what this did — let `fe90::`, `fea0::` and
 * `feb0::` through while blocking only the first sixteenth of the range it named.
 */
const isPrivateAddress = (address) => {
  if (isIP(address) === 6) {
    const plain = address.toLowerCase().replace(/^\[|\]$/g, '');

    return isPrivateV6(plain);
  }

  const parts = address.split('.').map(Number);

  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part))) return true;

  const [a, b, c] = parts;

  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 100 && b >= 64 && b <= 127) ||
    // 192.0.0.0/24 (IETF protocol assignments) and 192.0.2.0/24 (TEST-NET-1): neither routes
    (a === 192 && b === 0 && (c === 0 || c === 2)) ||
    // 198.51.100.0/24 and 203.0.113.0/24, the other two documentation ranges
    (a === 198 && b === 51 && c === 100) ||
    (a === 203 && b === 0 && c === 113) ||
    // 198.18.0.0/15, benchmarking
    (a === 198 && (b === 18 || b === 19)) ||
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

  /**
   * Drops a user from both maps at once.
   *
   * `lastPushAt` used to be added to and never removed from, so a server accumulated one entry per
   * user who had ever been pushed to and kept it for the life of the process. Small, but it is the
   * same two lines either way.
   */
  const forgetSubscriber = (userId) => {
    subscribers.delete(userId);
    lastPushAt.delete(userId);
  };

  const muted = (stored, channelId) =>
    Array.isArray(stored?.mutedChannels) && stored.mutedChannels.includes(channelId);

  /**
   * Posts the wake-up, and forgets an endpoint the relay says is gone.
   *
   * `410 Gone` and `404` are the distributor telling us this registration no longer exists — a
   * phone that uninstalled Shiver or changed distributor. Left alone it would be posted to forever,
   * so it is dropped from the row on the spot.
   *
   * The endpoint is re-validated and the redirect refused before any of that; see below.
   */
  const wake = async (userId, endpoint) => {
    // **Checked again, here, immediately before the request.** `register` checked it once, but a
    // hostname's records can change after it was stored, and the endpoint is then posted to on
    // every message for as long as it is registered — so a check that only ran at registration is
    // a check an attacker waits out. This is the one that decides whether the request is made.
    const allowed = await endpointAllowed(endpoint);

    if (!allowed.ok) {
      ctx.logger.debug(`Refusing to post to a push endpoint for user ${userId}: ${allowed.why}`);

      return;
    }

    const control = new AbortController();
    const timer = setTimeout(() => control.abort(), PUSH_TIMEOUT_MS);

    try {
      const response = await fetch(endpoint, {
        method: 'POST',
        signal: control.signal,
        // **Never followed.** `fetch` follows redirects by default, and every check above applies
        // to the URL the user registered rather than to wherever it points next — so a relay that
        // answers `307 Location: http://127.0.0.1:6379/` would have this server make that request,
        // with the method and body intact, to a service the user cannot reach themselves. That is
        // the whole of the server-side request forgery this file exists to prevent, arriving one
        // hop later. Shiver's own http calls refuse redirects for exactly this reason
        // (`desktop/src-tauri/src/login.rs`, `probe.rs`); this is the same rule.
        redirect: 'manual',
        // an empty body on purpose: see the note at the top of this file
        headers: { 'Content-Length': '0', 'TTL': '2419200' }
      });

      // A distributor answering a wake-up with a redirect is not something to chase. `manual`
      // surfaces it as an ordinary response, so it is reported rather than silently treated as
      // delivered.
      if (response.status >= 300 && response.status < 400) {
        ctx.logger.debug(
          `A push endpoint for user ${userId} answered ${response.status}; redirects are not followed`
        );

        return;
      }

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

  /**
   * Who should be woken for a message, and why each of the others should not.
   *
   * Asked of every subscriber **at once** rather than one after another. Each one costs a row read
   * and a permission check, and this runs on the server's message path — sequentially that is two
   * round trips per subscriber before the first push goes out, for every message on the server.
   *
   * The debounce is stamped here, at the moment a user is chosen, rather than after the whole set
   * has been worked out. Stamped afterwards it did not hold: several messages arriving inside the
   * window all read the old timestamp, all passed, and all pushed — which is the thing
   * `DEBOUNCE_MS` exists to stop, and it got easier to hit as the subscriber list grew.
   */
  const recipients = async (message) => {
    const now = Date.now();

    const considered = await Promise.all(
      [...subscribers].map(async (userId) => {
        // their own message, arriving back at them
        if (userId === message.userId) return null;

        const since = lastPushAt.get(userId) ?? 0;

        if (now - since < DEBOUNCE_MS) return null;

        const stored = await ctx.userData.get(userId).catch(() => null);
        const endpoints = endpointsFrom(stored?.pushEndpoints);

        if (!endpoints.length) return { userId, drop: true };

        // the user's own mute, kept by this same plugin, applied before the phone is disturbed
        if (muted(stored, message.channelId)) return null;

        const allowed = await ctx.permissions
          .userCanInChannel(userId, message.channelId, 'VIEW_CHANNEL')
          .catch(() => false);

        if (!allowed) return null;

        return { userId, endpoints };
      })
    );

    const wanted = [];

    for (const entry of considered) {
      if (!entry) continue;

      if (entry.drop) {
        forgetSubscriber(entry.userId);

        continue;
      }

      // claimed before the sends are issued, so a message arriving while they are in flight sees
      // this user as already pushed
      lastPushAt.set(entry.userId, now);

      wanted.push(entry);
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

    // not awaited: one slow relay must not hold up the others, and none of them may hold up the
    // message that caused this. `recipients` has already stamped the debounce for each of them.
    for (const { userId, endpoints } of wanted) {
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

    if (!endpoints.length) forgetSubscriber(userId);

    return endpoints;
  };

  /**
   * Learns, from a row somebody else has already read, whether this user can be woken.
   *
   * A taker rather than a loop of its own: `onLoad` reads each user's row once and offers it here
   * and to `statuses`, so the two features share one pass instead of walking every user twice.
   */
  const adopt = (userId, stored) => {
    if (endpointsFrom(stored?.pushEndpoints).length) subscribers.add(userId);
  };

  adopt.done = () => {
    if (subscribers.size) {
      ctx.logger.log(`Shiver push: ${subscribers.size} user(s) can be woken when they are away`);
    }
  };

  return {
    adopt,
    register,
    unregister,
    onMessage
  };
};
