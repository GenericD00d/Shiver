import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';
import type { ServerCheck } from '../types';

type Props = {
  onAdded: (id: string) => void;
  onCancel: () => void;
  canCancel: boolean;
};

export const AddServerPanel = ({ onAdded, onCancel, canCancel }: Props) => {
  const [origin, setOrigin] = useState('');
  const [identity, setIdentity] = useState('');
  const [password, setPassword] = useState('');
  const [accountLabel, setAccountLabel] = useState('');
  // the escape hatch for servers behind an identity provider, where Shiver cannot sign in itself
  const [signInHere, setSignInHere] = useState(true);
  // off by default: Shiver keeps as little as it can, and the session alone opens the server
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
      setPreview(await api.checkServer(origin, identity.trim() || null, password || null));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [origin, identity, password]);

  const handleAdd = useCallback(async () => {
    setBusy(true);
    setError(null);

    try {
      const entry = await api.addServer(
        origin,
        signInHere ? identity.trim() : undefined,
        signInHere ? password : undefined,
        accountLabel.trim() || undefined,
        signInHere
      );

      onAdded(entry.id);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [accountLabel, identity, onAdded, origin, password, signInHere]);

  const handleSubmit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();

      if (preview) {
        handleAdd();

        return;
      }

      handleCheck();
    },
    [handleAdd, handleCheck, preview]
  );

  const handleOriginChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    setOrigin(event.target.value);
    // the preview describes the address that was checked, so a further edit invalidates it
    setPreview(null);
  }, []);

  const handleLabelChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    setAccountLabel(event.target.value);
  }, []);

  const handleIdentityChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    setIdentity(event.target.value);
  }, []);

  const handlePasswordChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    setPassword(event.target.value);
  }, []);

  const handleSignInThereChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    setSignInHere(!event.target.checked);
  }, []);

  return (
    <div className="modal-backdrop">
      <form className="card" onSubmit={handleSubmit}>
        <h1>Add a server</h1>
        <p className="hint">
          Shiver signs in for you so the server opens straight into the app. Your password goes only
          to this server, and Shiver keeps it in your operating system's credential store so it can
          sign you in again when the session runs out — Sharkord's last a week and cannot be
          renewed. Take it back whenever you like from the server's own menu.
        </p>

        {error ? <p className="error">{error}</p> : null}

        <label className="field">
          <span>Server address</span>
          <input
            type="text"
            value={origin}
            onChange={handleOriginChange}
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
            <div>
              <strong>{preview.name}</strong>
              <small>{preview.origin}</small>

              {/* Three answers, and the third is not a failure: without a password Shiver has no
                  way to ask, and saying "no plugin" then would be a guess presented as a fact. */}
              {preview.plugin === undefined ? (
                <small className="plugin-unknown">
                  Sign in below to see whether it has the Shiver plugin
                </small>
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
                onChange={handleIdentityChange}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
              />
            </label>

            <label className="field">
              <span>Password</span>
              <input type="password" value={password} onChange={handlePasswordChange} />
            </label>
          </>
        ) : null}

        <label className="checkbox">
          <input type="checkbox" checked={!signInHere} onChange={handleSignInThereChange} />
          <span>
            Sign in on the server's own page instead
            <small>
              Use this for servers that sign you in through an identity provider, where Shiver cannot
              do it for you.
            </small>
          </span>
        </label>

        <label className="field">
          <span>Account label (optional)</span>
          <input
            type="text"
            value={accountLabel}
            onChange={handleLabelChange}
            placeholder="Defaults to your username"
          />
        </label>

        <div className="actions">
          {canCancel ? (
            <button type="button" className="ghost" onClick={onCancel} disabled={busy}>
              Cancel
            </button>
          ) : null}

          <button
            type="submit"
            className="primary"
            disabled={
              busy || !origin.trim() || (!!preview && signInHere && (!identity.trim() || !password))
            }
          >
            {preview ? 'Sign in and add' : 'Check server'}
          </button>
        </div>
      </form>
    </div>
  );
};
