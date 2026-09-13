import { useCallback, useState } from 'react';

import { api } from '../api';
import type { ServerEntry } from '../types';

type Props = {
  server: ServerEntry;
  /** whether Shiver is already keeping this server's password */
  remembered: boolean;
  onDone: () => void;
  onCancel: () => void;
};

/**
 * Signing an existing server in again.
 *
 * The way back from a session Shiver could not renew. Sharkord signs one for seven days and offers no
 * way to refresh it, so a server Shiver holds no password for eventually stops reporting — no badge,
 * no messages, nothing in the direct-message list — and this is what the user does about it.
 *
 * Ticking the box means it is the last time: Shiver keeps the password and signs in again by itself.
 * Unticking it on a server that has one throws that password away.
 */
export const SignInServer = ({ server, remembered, onDone, onCancel }: Props) => {
  const [identity, setIdentity] = useState(server.identity ?? '');
  const [password, setPassword] = useState('');
  const [remember, setRemember] = useState(remembered);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();

      if (busy) return;

      setBusy(true);
      setError(null);

      try {
        await api.signInServer(server.id, identity.trim(), password, remember);
        onDone();
      } catch (problem) {
        setError(String(problem));
      } finally {
        setBusy(false);
      }
    },
    [busy, identity, onDone, password, remember, server.id]
  );

  return (
    <form className="panel" onSubmit={submit}>
      <h1>Sign in to {server.name}</h1>

      <p className="hint">
        Sharkord expires a session after seven days and offers no way to renew one, so Shiver's watch
        of {server.origin.replace(/^https?:\/\//, '')} ends with it. Signing in gives Shiver a fresh
        session.
      </p>

      <label className="field">
        <span>Username or email</span>
        <input
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          value={identity}
          onChange={(event) => setIdentity(event.target.value)}
        />
      </label>

      <label className="field">
        <span>Password</span>
        <input
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
      </label>

      <label className="checkbox">
        <input
          type="checkbox"
          checked={remember}
          onChange={(event) => setRemember(event.target.checked)}
        />
        <span>
          Stay signed in on this device
          <small>
            Shiver keeps the password and signs in again by itself, so this server never goes quiet.
            Stored encrypted under a key the phone's Keystore holds and Shiver cannot read out.
            Leaving this unticked means signing in here again when the session next expires.
          </small>
        </span>
      </label>

      {error ? <p className="error">{error}</p> : null}

      <button
        type="submit"
        className="primary wide"
        disabled={busy || !identity.trim() || !password}
      >
        {busy ? 'Signing in…' : 'Sign in'}
      </button>

      <button type="button" className="ghost wide" onClick={onCancel}>
        Cancel
      </button>
    </form>
  );
};
