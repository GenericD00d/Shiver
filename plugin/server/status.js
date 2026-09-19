/**
 * Custom statuses: a line a user writes about themselves, shown beside their name.
 *
 * Sharkord has a `status`, but it is presence — `online`, `idle`, `offline`, set by the server from
 * whether a socket is connected. There is nothing a user can *write*, so this adds one rather than
 * reimplementing anything.
 *
 * It lives in the plugin rather than in Shiver's client for a reason worth stating: a status is only
 * useful if **other people see it**, and Shiver's client can only change what its own user sees. So
 * the text is stored on the server, in the same per-user row this plugin already keeps for muted
 * channels, and rendered through Sharkord's own `MEMBER_LIST_ITEM`, `USER_POPOVER` and
 * `USER_SETTINGS` plugin slots.
 * The consequence is that it works for anyone on a server with this plugin, whether they use Shiver or
 * an ordinary browser — which is more than a Shiver-side feature could have managed.
 *
 * Distribution is `ctx.push.toAll`, so a change reaches everyone already looking at the member list
 * without them refetching. A client that joins later asks once with `getStatuses`.
 */

/** Long enough for a sentence, short enough to sit beside a name without pushing it around. */
const MAX_STATUS = 100;

/** A stored value read back as a status: trimmed, capped, and single-line. */
export const statusFrom = (value) =>
  typeof value === 'string' ? value.replace(/\s+/g, ' ').trim().slice(0, MAX_STATUS) : '';

export const createStatuses = (ctx) => {
  /**
   * Everyone's status, so the member list costs nothing to draw.
   *
   * Read once at load and kept in step by writes. The alternative — a row read per member per
   * render — would turn opening the member list into one database query per person in it.
   */
  const statuses = new Map();

  /**
   * Takes this user's status out of a row somebody else has already read.
   *
   * See `primeFromUserRows` in `index.js`: push wants the same row, and reading it twice at every
   * load was two sequential passes over every user on the server for no reason.
   */
  const adopt = (userId, stored) => {
    const status = statusFrom(stored?.status);

    if (status) statuses.set(userId, status);
  };

  adopt.done = () => {
    if (statuses.size) ctx.logger.log(`Shiver: ${statuses.size} user(s) have a status set`);
  };

  /**
   * Sets the caller's own status.
   *
   * The invoker's id is used and any id in the payload ignored, so there is no shape of call that
   * writes a status onto somebody else's name. An empty string clears it.
   */
  const set = async (userId, value) => {
    const status = statusFrom(value);
    const stored = await ctx.userData.get(userId);

    // the row is replaced wholesale by a write, so it is read first and spread — the same care the
    // muted channels need, and for the same reason
    await ctx.userData.set(userId, { ...stored, status });

    if (status) {
      statuses.set(userId, status);
    } else {
      statuses.delete(userId);
    }

    // Told to everyone, because a status is for other people. `toAll` rather than a refetch: the
    // member list is already open on other screens and should simply change.
    ctx.push.toAll({ kind: 'status', userId, status });

    return { status };
  };

  const all = () => ({
    statuses: [...statuses.entries()].map(([userId, status]) => ({ userId, status }))
  });

  /** A user who has left takes their status with them. */
  const forget = (userId) => {
    if (statuses.delete(userId)) {
      ctx.push.toAll({ kind: 'status', userId, status: '' });
    }
  };

  return { adopt, set, all, forget };
};
