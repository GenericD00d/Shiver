/**
 * Shiver companion plugin, server half.
 *
 * Keeps each user's Shiver settings for this server, so they follow the user to every device instead
 * of living on one machine. Today that is the muted channel list.
 *
 * Since Sharkord 0.0.25 the host stores that itself: every plugin gets a per-user row, size-capped,
 * removed with the user and with the plugin. Shiver used to keep a JSON file beside this code and
 * register two actions to reach it — the storage, the serialised writes, the atomic rename and the
 * cleanup hook are all the host's job now, and it does the last one better than we did.
 *
 * So what is left here is the one thing the host cannot do: bring the old file forward. That, and
 * enabling the client bundle, which is the relay Shiver's bridge actually talks to.
 */

import { readFile, unlink } from 'node:fs/promises';
import path from 'node:path';

import { createPush } from './push.js';
import { createStatuses } from './status.js';
import { installFileNaming } from './files.js';

/** The file Shiver kept its settings in before the host had somewhere to put them. */
const STORE_FILE = 'user-settings.json';

/** Guards against an old file carrying an unbounded list into the host's row. */
const MAX_MUTED_CHANNELS = 500;

/** A stored value read back as a mute list, keeping only what a channel id can be. */
const mutedFrom = (value) =>
  Array.isArray(value)
    ? [...new Set(value.filter((id) => Number.isInteger(id) && id > 0))].slice(0, MAX_MUTED_CHANNELS)
    : [];

/**
 * Moves settings out of the file Shiver used to keep and into the host's per-user storage.
 *
 * Merged rather than moved: a user who has already muted something through the new storage keeps
 * what they have, and the file only fills in for those who have nothing yet. The file is deleted
 * once it has been read, so this happens once and the next load has nothing to do.
 *
 * Both of the places it could be is checked. `ctx.path` is where it lived before 0.0.25; `dataPath`
 * is where a brief later version of this plugin put it, before the host's own storage made both
 * unnecessary.
 */
const adoptOldStore = async (ctx) => {
  const places = [...new Set([path.join(ctx.dataPath, STORE_FILE), path.join(ctx.path, STORE_FILE)])];

  for (const place of places) {
    let entries;

    try {
      const parsed = JSON.parse(await readFile(place, 'utf-8'));

      if (!parsed || typeof parsed !== 'object') continue;

      entries = Object.entries(parsed);
    } catch {
      // no file here, which is the ordinary case
      continue;
    }

    let moved = 0;

    for (const [id, entry] of entries) {
      const userId = Number(id);
      const mutedChannels = mutedFrom(entry?.mutedChannels);

      if (!Number.isInteger(userId) || !mutedChannels.length) continue;

      try {
        const stored = await ctx.userData.get(userId);

        // already saying something for this user, so the file is the older story
        if (mutedFrom(stored?.mutedChannels).length) continue;

        await ctx.userData.set(userId, { ...stored, mutedChannels });

        moved += 1;
      } catch (error) {
        // a user who has since been deleted, most likely: their row would not be storable anyway
        ctx.logger.debug(`Could not carry over Shiver settings for user ${userId}: ${error?.message}`);
      }
    }

    await unlink(place).catch(() => {});

    ctx.logger.log(`Moved Shiver settings for ${moved} user(s) into Sharkord's own storage`);
  }
};

/**
 * Reads every user's row once and hands each one to whoever wants it.
 *
 * The cost is one read per user on a server that has never used either feature, which is the price
 * of there being no "who has data" query. It is paid at load rather than per message, and a server
 * with nothing stored ends up doing no per-message work at all.
 */
const primeFromUserRows = async (ctx, takers) => {
  let users;

  try {
    users = await ctx.users.list();
  } catch (error) {
    ctx.logger.debug(`Could not list users to read Shiver's stored settings: ${error?.message}`);

    return;
  }

  const rows = await Promise.all(
    users.map(async (user) => [user.id, await ctx.userData.get(user.id).catch(() => null)])
  );

  for (const [userId, stored] of rows) {
    for (const take of takers) take(userId, stored);
  }

  for (const take of takers) take.done?.();
};

/** Subscriptions to drop on unload, so a reload does not end up with two of everything. */
let stop = [];

const onLoad = async (ctx) => {
  await adoptOldStore(ctx);

  // Without this the client bundle is never listed, so Sharkord never imports it, and the relay
  // Shiver talks to would not exist. The bundle renders no ui of its own.
  ctx.ui.enable();

  installFileNaming(ctx);

  const push = createPush(ctx);
  const statuses = createStatuses(ctx);

  // **One pass over the rows, not two.** Both of these want the same row of every user, and each
  // used to walk `ctx.users.list()` and await a read per user on its own — so a server paid for
  // every row twice at every load, sequentially, for two maps that could be filled together.
  await primeFromUserRows(ctx, [push.adopt, statuses.adopt]);

  /**
   * A line the user writes about themselves, shown beside their name to everyone.
   *
   * Sharkord's own `status` is presence — online, idle, offline — and is the server's to set. This
   * is the other kind, and it has to live here rather than in a client because the point of it is
   * that other people see it.
   */
  ctx.actions.register({
    name: 'setStatus',
    description: 'Set your own status line',
    executes: async (invoker, payload) => statuses.set(invoker.userId, payload?.status)
  });

  ctx.actions.register({
    name: 'getStatuses',
    description: 'Everyone who has a status set',
    executes: async () => statuses.all()
  });

  /**
   * Registering a phone to be woken.
   *
   * An action rather than a write from the client relay, because the endpoint has to be *checked*
   * before this server will ever post to it — it is a url supplied by a user that this server would
   * then fetch, which is server-side request forgery if nobody looks. The check has to run here;
   * the browser cannot do it and could not be trusted to.
   *
   * The invoker's own id is used and the payload's is ignored, so there is no shape of call that
   * registers an endpoint against somebody else's account.
   */
  ctx.actions.register({
    name: 'setPushEndpoint',
    description: 'Register a UnifiedPush endpoint so Shiver can wake this device',
    executes: async (invoker, payload) => ({
      endpoints: await push.register(invoker.userId, String(payload?.endpoint ?? ''))
    })
  });

  ctx.actions.register({
    name: 'clearPushEndpoint',
    description: 'Stop waking a device, or all of them when no endpoint is named',
    executes: async (invoker, payload) => ({
      endpoints: await push.unregister(
        invoker.userId,
        typeof payload?.endpoint === 'string' ? payload.endpoint : undefined
      )
    })
  });

  // No presence listener any more, on purpose — see the note at the top of `push.js`. Sharkord's
  // `user:left` means "this account's last socket closed", and Shiver itself holds one per server per
  // device, so presence here only ever meant "Shiver is installed somewhere and running".
  stop = [
    ctx.events.on('message:created', (message) => {
      void push.onMessage(message);
    }),
    // a deleted user should not go on describing themselves in the member list
    ctx.events.on('user:deleted', ({ userId }) => statuses.forget(userId))
  ];

  ctx.logger.log('Shiver plugin loaded');
};

const onUnload = async (ctx) => {
  for (const off of stop) {
    try {
      off?.();
    } catch {
      // an unsubscribe that throws is not worth failing an unload over
    }
  }

  stop = [];

  ctx.logger.log('Shiver plugin unloaded');
};

export { onLoad, onUnload };
