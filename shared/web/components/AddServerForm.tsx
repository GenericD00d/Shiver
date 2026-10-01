import { type ReactNode, useCallback, useState } from 'react';

import { errorMessage } from '../errors';
import type { ServerCheck } from '../types';
import './ui.css';

/** What the form hands over to be added. Without credentials the user signs in on the server's own page. */
export type NewServer = {
  origin: string;
  identity: string | null;
  password: string | null;
  accountLabel: string | null;
};

type Props = {
  /** where Shiver keeps the password, in this platform's words */
  hint: ReactNode;
  /** looks the server up and, when credentials are given, says whether it has the Shiver plugin */
  check: (origin: string, identity: string | null, password: string | null) => Promise<ServerCheck>;
  /** signs in (when credentials are given) and adds it; a failure is shown on the form */
  add: (server: NewServer) => Promise<void>;
  /** null when there is nowhere to go back to */
  onCancel: (() => void) | null;
};

/**
 * Adds a server in two steps: the address is checked first and the server's own name and logo
 * shown, so the user sees who they are giving a password to, then it signs in and adds it.
 */
export const AddServerForm = ({ hint, check, add, onCancel }: Props) => {
  const [origin, setOrigin] = useState('');
  const [identity, setIdentity] = useState('');
  const [password, setPassword] = useState('');
  const [accountLabel, setAccountLabel] = useState('');
  // the escape hatch for servers behind an identity provider, where Shiver cannot sign in itself
  const [signInHere, setSignInHere] = useState(true);
  const [preview, setPreview] = useState<ServerCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const handleCheck = useCallback(async () => {
    setBusy(true);
    setError(null);
    setPreview(null);

    try {
      // credentials are handed over when they are there: the plugin cannot be seen without a
      // session, so a check with an empty password can only answer what `/info` answers
      setPreview(await check(origin.trim(), signInHere ? identity.trim() || null : null, signInHere ? password || null : null));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [check, identity, origin, password, signInHere]);

  const handleAdd = useCallback(async () => {
    setBusy(true);
    setError(null);

    try {
      await add({
        origin: origin.trim(),
        identity: signInHere ? identity.trim() || null : null,
        password: signInHere ? password || null : null,
        accountLabel: accountLabel.trim() || null
      });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [accountLabel, add, identity, origin, password, signInHere]);

  const handleSubmit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();

      if (busy) return;

      // one button, two steps: it checks the address, and once that has answered it adds
      void (preview ? handleAdd() : handleCheck());
    },
    [busy, handleAdd, handleCheck, preview]
  );

  return (
    <form className="card" onSubmit={handleSubmit}>
      <h1>Add a server</h1>
      <p className="hint">{hint}</p>

      {error ? <p className="error">{error}</p> : null}

      <label className="field">
        <span>Server address</span>
        {/* deliberately not `type="url"`: the browser then applies its own validity check before
            the form will submit, and that check rejects a bare host — which is the common way to
            type an address, what the placeholder shows, and what `normalize_origin` is built to
            accept. Validation belongs in rust, where it is also stricter: it turns down embedded
            credentials and anything but https, neither of which the browser objects to. */}
        <input
          type="text"
          value={origin}
          onChange={(event) => {
            setOrigin(event.target.value);
            // the preview describes the address that was checked, so a further edit invalidates it
            setPreview(null);
          }}
          placeholder="chat.example.com"
          autoFocus
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          inputMode="url"
        />
      </label>

      {preview ? (
        <div className="preview">
          {preview.iconUrl ? (
            <img src={preview.iconUrl} alt="" />
          ) : (
            <span className="fallback">{preview.name.slice(0, 1).toUpperCase()}</span>
          )}

          <div className="preview-text">
            <strong>{preview.name}</strong>
            <small>{preview.origin}</small>

            {/* Three answers, and the third is not a failure: without a password Shiver has no
                way to ask, and saying "no plugin" then would be a guess presented as a fact. */}
            {preview.plugin === undefined ? (
              <small className="plugin-unknown">Sign in below to see whether it has the Shiver plugin</small>
            ) : preview.plugin ? (
              <small className="has-plugin">{'✓'} Shiver plugin {preview.plugin}</small>
            ) : (
              <small className="no-plugin">{'✗'} No Shiver plugin</small>
            )}
          </div>
        </div>
      ) : null}

      {signInHere ? (
        <>
          <label className="field">
            <span>Username</span>
            <input
              type="text"
              value={identity}
              onChange={(event) => setIdentity(event.target.value)}
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
              autoComplete="current-password"
            />
          </label>
        </>
      ) : null}

      <label className="checkbox">
        <input type="checkbox" checked={!signInHere} onChange={(event) => setSignInHere(!event.target.checked)} />
        <span>
          Sign in on the server's own page instead
          <small>
            Use this for servers that sign you in through an identity provider, where Shiver cannot do it for
            you.
          </small>
        </span>
      </label>

      <label className="field">
        <span>Account label (optional)</span>
        <input
          type="text"
          value={accountLabel}
          onChange={(event) => setAccountLabel(event.target.value)}
          placeholder="Defaults to your username"
        />
      </label>

      <div className="actions">
        {onCancel ? (
          <button type="button" className="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
        ) : null}

        <button
          type="submit"
          className="primary"
          disabled={busy || !origin.trim() || (!!preview && signInHere && (!identity.trim() || !password))}
        >
          {busy ? (preview ? 'Adding…' : 'Checking…') : preview ? 'Sign in and add' : 'Check server'}
        </button>
      </div>
    </form>
  );
};
