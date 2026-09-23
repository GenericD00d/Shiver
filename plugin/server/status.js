/**
 * Custom statuses: a line each user writes about themselves, shown beside their name via
 * Sharkord's plugin slots and broadcast with `ctx.push.toAll` when it changes.
 */

import { createLimiter } from './rows.js';

export const MAX_STATUS = 100;
const STATUS_LIMIT = 10;

/** Bidi overrides, zero-width and other invisible format characters: only useful for spoofing. */
const INVISIBLE =
  /[­͏؜ᅟᅠ឴឵᠋-᠏​-‏‪-‮⁠-⁯ㅤ︀-️﻿ﾠ\u{e0000}-\u{e0fff}]/gu;

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
