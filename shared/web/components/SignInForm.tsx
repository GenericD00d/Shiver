import { type ReactNode, useCallback, useState } from 'react';

import { errorMessage } from '../errors';
import './ui.css';

type Props = {
  serverName: string;
  /** who Shiver signs in as, when it knows */
  identity: string | null;
  /** why this is asked and what is kept, in this platform's words */
  hint: ReactNode;
  /** what keeping the password means on this platform, under the checkbox */
  rememberHint: ReactNode;
  /** whether the checkbox starts ticked: Shiver already keeps this server's password */
  remembered: boolean;
  /** signs in; a failure is shown on the form, a success moves the caller on */
  onSubmit: (identity: string, password: string, rememberPassword: boolean) => Promise<void>;
  onCancel: () => void;
};

/**
 * Signs a server in through Shiver, so it holds the session and can renew it (a sign-in on the
 * server's own page never gives Shiver the password).
 */
export const SignInForm = ({ serverName, identity: known, hint, rememberHint, remembered, onSubmit, onCancel }: Props) => {
  // Shiver usually knows who the entry is for, and is only missing the password
  const [identity, setIdentity] = useState(known ?? '');
  const [password, setPassword] = useState('');
  // answered again here rather than carried over, so unticking it drops a password kept from an
  // earlier sign-in rather than only changing what happens next time
  const [remember, setRemember] = useState(remembered);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();

      if (!identity.trim() || !password || busy) return;

      setBusy(true);
      setError(null);

      try {
        await onSubmit(identity.trim(), password, remember);
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setBusy(false);
      }
    },
    [busy, identity, onSubmit, password, remember]
  );

  return (
    <form className="card" onSubmit={handleSubmit}>
      <h1>Sign in to {serverName}</h1>
      <p className="hint">{hint}</p>

      <label className="field">
        <span>Identity</span>
        <input
          type="text"
          value={identity}
          onChange={(event) => setIdentity(event.target.value)}
          autoFocus={!known}
          autoComplete="username"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
        />
      </label>

      <label className="field">
        <span>Password</span>
        <input
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          autoFocus={!!known}
          autoComplete="current-password"
        />
      </label>

      <label className="checkbox">
        <input type="checkbox" checked={remember} onChange={(event) => setRemember(event.target.checked)} />
        <span>
          Keep my password for this server
          <small>{rememberHint}</small>
        </span>
      </label>

      {error ? <p className="error">{error}</p> : null}

      <div className="actions">
        <button type="button" className="ghost" onClick={onCancel} disabled={busy}>
          Cancel
        </button>
        <button type="submit" className="primary" disabled={busy || !identity.trim() || !password}>
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
      </div>
    </form>
  );
};
