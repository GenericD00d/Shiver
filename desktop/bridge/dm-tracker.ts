/**
 * This server's DM list for the core: each conversation's latest message, sent or received, as the
 * page's connection reports it, and the list redrawn only when its channels, users, a newly seen
 * message or a closed conversation changed it. A list that differs from the last one handed over
 * waits for the next drain (`take`).
 */

import { closedDms } from '../../shared/web/bridge/messages';
import { type DmChannel, readDms, type SharkordState } from '../../shared/web/bridge/sharkord';

export function createDmTracker(origin: string) {
  /** each channel's latest message */
  const lastSeen = new Map<number, number>();
  let lastSeenVersion = 0;
  let state: SharkordState = {};
  /** nothing is drawn before the page's document is ready */
  let started = false;
  let inputs: unknown[] = [];
  let signature = '';
  let changed: DmChannel[] | null = null;

  const refresh = () => {
    if (!started) return;

    const next = [state.channels, state.users, state.ownUserId, lastSeenVersion];

    if (next.every((input, index) => input === inputs[index])) return;

    inputs = next;

    const list = readDms(origin, state, lastSeen, closedDms());
    // avatar urls carry expiring tokens, so they are left out of the comparison
    const drawn = JSON.stringify(list.map(({ channelId, name, lastMessageAt }) => [channelId, name, lastMessageAt]));

    if (drawn !== signature) {
      signature = drawn;
      changed = list;
    }
  };

  return {
    /** A message in `channelId` at `at`, moving that conversation up if it is newer. */
    noteMessage(channelId: number, at: number) {
      if (at <= (lastSeen.get(channelId) ?? 0)) return false;

      lastSeen.set(channelId, at);
      lastSeenVersion += 1;
      refresh();

      return true;
    },
    /** The user closed or reopened a conversation. */
    closedChanged() {
      lastSeenVersion += 1;
      refresh();
    },
    /** The page's document is ready: the list can be drawn. */
    start() {
      started = true;
    },
    /** Sharkord's store changed. */
    update(next: SharkordState) {
      state = next;
      refresh();
    },
    /** The list, if it changed since the last call. */
    take() {
      const taken = changed;

      changed = null;

      return taken;
    }
  };
}
