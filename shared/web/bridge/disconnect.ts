/**
 * Tells the user's own Disconnect apart from a session the server refused. Both end at Sharkord's
 * connect form, but after a refusal Shiver signs back in, and after a Disconnect it must wait until
 * the user signs in again (docs/decisions.md). The only difference the page shows is the choice in
 * Sharkord's server menu, so that is noted, and a connect form soon after counts as its result.
 *
 * The page can claim this whenever it likes. All the claim does is stop Shiver signing this one
 * entry in, which a server could do anyway by refusing the password.
 */

import { SERVER_MENU_DISCONNECT } from './sharkord';

/**
 * How long after choosing Disconnect a connect form counts as its result. Sharkord asks for
 * confirmation first, which the user may take a while over.
 */
export const DISCONNECT_WINDOW_MS = 120_000;

export function createDisconnectWatch(clock: () => number = Date.now) {
  let chosenAt: number | null = null;

  return {
    /** The menu item was chosen; the confirmation that follows is still the user's to answer. */
    chosen() {
      chosenAt = clock();
    },

    /**
     * Whether Disconnect was chosen within the window and its connect form hasn't been taken yet:
     * the confirmation is up, or its answer is on its way. Nothing should sign the page in then.
     */
    pending() {
      const elapsed = chosenAt === null ? -1 : clock() - chosenAt;

      return elapsed >= 0 && elapsed < DISCONNECT_WINDOW_MS;
    },

    /** Whether the page has just been disconnected by the user. Answers yes once per choice. */
    take(atConnectForm: boolean) {
      if (chosenAt === null) return false;

      const elapsed = clock() - chosenAt;

      // too long ago (or the clock went back): the user cancelled, and whatever ends the session
      // now is something else
      if (elapsed < 0 || elapsed >= DISCONNECT_WINDOW_MS) {
        chosenAt = null;

        return false;
      }

      if (!atConnectForm) return false;

      chosenAt = null;

      return true;
    }
  };
}

/**
 * Calls back whenever Sharkord's Disconnect item is chosen. Choosing it with the keyboard clicks it
 * too (the menu's items turn Enter and Space into a click), so one listener sees both.
 */
export function watchDisconnectChoice(onChosen: () => void) {
  document.addEventListener(
    'click',
    (event) => {
      const target = event.target;

      if (target instanceof Element && target.closest(SERVER_MENU_DISCONNECT)) onChosen();
    },
    true
  );
}
