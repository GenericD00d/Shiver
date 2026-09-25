/**
 * Shiver companion plugin, server half: registers the actions the client relay calls, migrates the
 * settings file older versions kept, and wires message events to push.
 *
 * Every action acts on `invoker.userId` from the session, never an id in the payload, and every
 * write is rate limited per user.
 */

import { readFile, unlink } from 'node:fs/promises';
import path from 'node:path';

import { installFileNaming } from './files.js';
import { createPush } from './push.js';
import { createLimiter, createRows } from './rows.js';
import { createSettings, mutedFrom } from './settings.js';
import { createStatuses } from './status.js';

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
