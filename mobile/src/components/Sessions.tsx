import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';

type Props = {
  /** re-reads which servers are signed in, since clearing changes it */
  onCleared: () => void;
};

/**
 * Explains that Shiver keeps each server's session (encrypted, Keystore key) to watch servers in
 * the background, and offers to forget them all.
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
        A session per watched server, encrypted on this device, so badges and notifications arrive
        for servers you are not looking at.
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
