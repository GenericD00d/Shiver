import { useCallback, useState } from 'react';

import type { ServerEntry } from '../types';

type Props = {
  server: ServerEntry | undefined;
  onSignIn: (
    id: string,
    identity: string,
    password: string,
    rememberPassword: boolean
  ) => Promise<void>;
  onCancel: () => void;
};

/**
 * Signs a rail entry in through Shiver, so it holds the session and can renew it (a sign-in on the
 * server's own page never gives Shiver the password).
 */
export const SignInPanel = ({ server, onSignIn, onCancel }: Props) => {
  // Shiver usually knows who the entry is for, and is only missing the password
  const [identity, setIdentity] = useState(server?.identity ?? '');
  const [password, setPassword] = useState('');
  // asked every time, and off unless the user says so — the same opt-in the add panel has. Answered
  // again here rather than carried over, so unticking it drops a password kept from an earlier
  // sign-in rather than only changing what happens next time.
  const [rememberPassword, setRememberPassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();

      if (!server || !identity.trim() || !password || busy) return;

      setBusy(true);
      setError(null);

      try {
        await onSignIn(server.id, identity.trim(), password, rememberPassword);
      } catch (cause) {
        setError(typeof cause === 'string' ? cause : 'Could not sign in');
        setBusy(false);
      }
    },
    [busy, identity, onSignIn, password, rememberPassword, server]
  );

  if (!server) return null;

  return (
    <div className="modal-backdrop">
      <form className="card" onSubmit={handleSubmit}>
        <h1>Sign in to {server.name}</h1>
        <p className="hint">
          Shiver keeps the session in your device&apos;s keychain and signs you in, so this server
          opens straight into the app instead of its login page. Your password is kept only if you
          ask below.
        </p>

        <label className="field">
          <span>Identity</span>
          <input
            type="text"
            value={identity}
            onChange={(event) => setIdentity(event.target.value)}
            autoFocus={!server?.identity}
            spellCheck={false}
          />
        </label>

        <label className="field">
          <span>Password</span>
          <input
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoFocus={!!server?.identity}
          />
        </label>

        <label className="checkbox">
          <input
            type="checkbox"
            checked={rememberPassword}
            onChange={(event) => setRememberPassword(event.target.checked)}
          />
          <span>
            Keep my password for this server
            <small>
              Stored in your operating system&apos;s credential store, so Shiver can sign in again by
              itself when the session expires — which it does every seven days. Left unticked, Shiver
              keeps only the session and asks you again when it runs out.
            </small>
          </span>
        </label>

        {error ? <p className="error">{error}</p> : null}

        <div className="actions">
          <button type="button" className="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button
            type="submit"
            className="primary"
            disabled={busy || !identity.trim() || !password}
          >
            {busy ? 'Signing in…' : 'Sign in'}
          </button>
        </div>
      </form>
    </div>
  );
};
