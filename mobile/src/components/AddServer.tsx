import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';
import type { ServerCheck } from '../types';

type Props = {
  onAdded: () => void;
};

/**
 * Adds a server in two steps: the address is checked first and the server's own name and logo
 * shown, so the user sees who they are giving a password to. Leaving the credentials empty adds the
 * server for signing in on its own page (needed behind an identity provider).
 */
export const AddServer = ({ onAdded }: Props) => {
  const [address, setAddress] = useState('');
  const [identity, setIdentity] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** what the server said about itself, once checked; null until then */
  const [preview, setPreview] = useState<ServerCheck | null>(null);

  const check = useCallback(async () => {
    if (!address.trim() || busy) return;

    setBusy(true);
    setError(null);

    try {
      // credentials are handed over when they are there: the plugin cannot be seen without a
      // session, so a check with an empty password can only answer what `/info` answers
      setPreview(
        await api.checkServer(
          address.trim(),
          identity.trim() || null,
          password || null
        )
      );
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [address, busy, identity, password]);

  const add = useCallback(async () => {
    if (!address.trim() || busy) return;

    setBusy(true);
    setError(null);

    try {
      await api.addServer(
        address.trim(),
        identity.trim() || null,
        password || null,
        !!password
      );
      onAdded();
    } catch (cause) {
      setError(errorMessage(cause));
      setBusy(false);
    }
  }, [address, busy, identity, onAdded, password]);

  return (
    <form
      className="panel"
      onSubmit={(event) => {
        event.preventDefault();

        // one button, two steps: it checks the address, and once that has answered it adds
        void (preview ? add() : check());
      }}
    >
      <label className="field">
        <span>Server address</span>
        {/* deliberately not `type="url"`: the browser then applies its own validity check before
            the form will submit, and that check rejects a bare host — which is the common way to
            type an address, what the placeholder shows, and what `normalize_origin` is built to
            accept. Validation belongs in rust, where it is also stricter: it turns down embedded
            credentials and anything but https, neither of which the browser objects to. */}
        <input
          type="text"
          inputMode="url"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          placeholder="chat.example.com"
          value={address}
          // the preview describes the address that was checked, so a further edit invalidates it
          onChange={(event) => {
            setAddress(event.target.value);
            setPreview(null);
          }}
        />
      </label>

      {preview ? (
        <div className="preview">
          {preview.iconUrl ? (
            <img src={preview.iconUrl} alt="" />
          ) : (
            <span className="fallback">{preview.name.slice(0, 1).toUpperCase()}</span>
          )}

          <span className="preview-text">
            <strong>{preview.name}</strong>
            <small>{preview.origin}</small>

            {/* Three answers, and the third is not a failure: without a password Shiver has no way
                to ask, and saying "no plugin" then would be a guess presented as a fact. */}
            {preview.plugin === undefined ? (
              <small className="plugin-unknown">
                Sign in below to see whether it has the Shiver plugin
              </small>
            ) : preview.plugin ? (
              <small className="has-plugin">{'✓'} Shiver plugin {preview.plugin}</small>
            ) : (
              <small className="no-plugin">{'✗'} No Shiver plugin</small>
            )}
          </span>
        </div>
      ) : null}

      <label className="field">
        <span>Username</span>
        <input
          type="text"
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="username"
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


      <p className="hint">
        Shiver signs in for you, so the server opens straight into the app. Your password goes only to
        this server, and both it and the session are kept in Android's encrypted store, under a key
        the phone's Keystore holds — so Shiver can sign you in again when the session runs out, which
        Sharkord makes it do every seven days. You can take the password back at any time by holding
        the server in the rail. Leave both empty to sign in on the server's own page instead, which
        is the only thing that works for a server behind an identity provider.
      </p>

      {error ? <p className="error">{error}</p> : null}

      <button type="submit" className="primary wide" disabled={busy || !address.trim()}>
        {busy ? (preview ? 'Adding…' : 'Checking…') : preview ? 'Sign in and add' : 'Check server'}
      </button>
    </form>
  );
};
