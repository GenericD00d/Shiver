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
