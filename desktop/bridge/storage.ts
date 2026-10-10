/** Seeding the page's storage before Sharkord reads it: the session, and its notification switches. */

import { notificationFlags } from '../../shared/web/bridge/sharkord';
import { AUTO_LOGIN, AUTO_LOGIN_TOKEN, installSessionShim } from '../../shared/web/session';
import type { NotifyLevel } from '../../shared/web/types';

/**
 * Serves Shiver's session to Sharkord's auto-login from memory (see `shared/web/session.ts`).
 * Without a session, any copy an older Shiver left in storage is removed so a log out sticks.
 */
export function seedSession(token: string | null) {
  if (token) {
    installSessionShim(token);

    return;
  }

  try {
    localStorage.removeItem(AUTO_LOGIN);
    localStorage.removeItem(AUTO_LOGIN_TOKEN);
  } catch {
    // storage blocked: the page shows its own login
  }
}

/**
 * Sharkord settings Shiver depends on: rejoining the last channel on connect, and its notification
 * switches (see `applyNotifyLevel`).
 */
export function seedDefaults(level: NotifyLevel) {
  writeStorage({ 'sharkord-auto-join-last-channel': 'true' }, false);
  applyNotifyLevel(level, false);
}

/** The level Shiver last wrote Sharkord's switches for, in the server's own storage. */
const NOTIFY_MARK = 'shiver-notify-level';

/**
 * Sharkord's notification switches for a level (Shiver's feed is built from its notifications). At
 * "all" they are written only when unset, so choices made in Sharkord's own settings stand, unless
 * Shiver wrote them for another level; any other level is Shiver's to keep, so it is written every
 * load. `force` writes them regardless (a change the user just made).
 */
export function applyNotifyLevel(level: NotifyLevel, force: boolean) {
  let mark: string | null = null;

  try {
    mark = localStorage.getItem(NOTIFY_MARK);
  } catch {
    // storage blocked
  }

  writeStorage(notificationFlags(level), force || level !== 'all' || (mark !== null && mark !== level));
  writeStorage({ [NOTIFY_MARK]: level }, true);
}

function writeStorage(values: Record<string, string>, overwrite: boolean) {
  try {
    for (const [key, value] of Object.entries(values)) {
      if (overwrite || localStorage.getItem(key) === null) localStorage.setItem(key, value);
    }
  } catch {
    // storage blocked
  }
}
