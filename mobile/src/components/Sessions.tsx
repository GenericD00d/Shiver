import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';

type Props = {
  /** re-reads which servers are signed in, since clearing changes it */
  onCleared: () => void;
};

/**
 * What Shiver is holding, said out loud, and the way to end it.
 *
 * Shiver watches the servers you are not looking at over its own connections, which it needs a
 * session for — so it borrows the one your own sign-in produced and keeps it encrypted, master key
 * in the Android Keystore. That is what makes the unread badges and notifications work at all, and
 * what makes them work from a cold start rather than only after you have opened each server.
 *
 * It is worth saying plainly rather than leaving to be discovered, and worth being refusable. A
 * session found only in the page's `sessionStorage` — someone who did not tick Sharkord's own
 * "Login automatically" — is already used for the run and never stored; this is for the rest.
 */
export const Sessions = ({ onCleared }: Props) => {
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const clear = useCallback(async () => {
    setBusy(true);
    setError(null);

    try {
      await api.forgetSessions();
      setDone(true);
      onCleared();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [onCleared]);

  return (
    <>
      <p className="hint">
        Shiver keeps a session for each server it watches, encrypted on this device, so unread counts
        and notifications arrive for servers you are not looking at. Servers where you did not ask
        to stay signed in are watched only while Shiver is running, and nothing is stored for them.
      </p>

      {error ? <p className="error">{error}</p> : null}

      {done ? (
        <p className="hint">
          Cleared. Badges and notifications will be quiet until you open each server again.
        </p>
      ) : null}

      <button type="button" className="ghost wide" disabled={busy} onClick={() => void clear()}>
        {busy ? 'Clearing…' : 'Forget stored sessions'}
      </button>
    </>
  );
};
