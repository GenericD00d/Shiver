/**
 * The page's shortcuts on their way to the core, and the core's answer on its way back. The client
 * awaits each `setHotkeys`; the latest set waits for the next drain (`take`), and the core answers
 * the set it drained (`answer`, through `__SHIVER_HOTKEYS_SET__`). A set replaced before it was
 * drained, or one drained before the core answered the previous one, is answered with no errors.
 */

import type { HotkeyBinding, HotkeyError } from './desktop-api';

type Answer = (errors: HotkeyError[]) => void;

export function createHotkeyHandoff() {
  /** none at first, so a reload clears what the page held before */
  let waiting: HotkeyBinding[] | null = [];
  let answerWaiting: Answer | null = null;
  let answerDrained: Answer | null = null;

  return {
    /** The client's new shortcuts; resolves with what the core could not register. */
    set(bindings: HotkeyBinding[]): Promise<HotkeyError[]> {
      answerWaiting?.([]);
      waiting = bindings;

      return new Promise((resolve) => {
        answerWaiting = resolve;
      });
    },
    /** The shortcuts set since the last drain, if any. */
    take() {
      const taken = waiting;

      if (taken !== null) {
        waiting = null;
        answerDrained?.([]);
        answerDrained = answerWaiting;
        answerWaiting = null;
      }

      return taken;
    },
    /** The core's answer to the set it drained last. */
    answer(errors: HotkeyError[]) {
      const answer = answerDrained;

      answerDrained = null;
      answer?.(errors);
    }
  };
}
