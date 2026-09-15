import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';
import type { ServerInfo } from '../types';

type Props = {
  onAdded: () => void;
};

/**
 * Adds a server, signing in on the way.
 *
 * **Two steps, as desktop has always had.** This was one step for a while, on the reasoning that a
 * button whose only job is to prove the address works earns nothing — adding checks the address
 * anyway, and a server that does not answer still fails. What changed is what the first step is
 * *for*: it now shows the server's own name and logo before any credentials are typed, so the
 * person can see they are about to hand a password to the server they meant. That is worth a tap.
 *
 * What it still cannot tell them is whether Shiver's companion plugin is installed. Nothing says so
 * without a session — `/info` does not mention plugins and `plugins.get` answers only an admin — so
 * the answer arrives on the first connection and is shown in the server list instead.
 *
 * The password goes to the server named above it and nowhere else, and what comes back is a session
 * kept in Android's encrypted store. Leaving both fields empty is a real choice rather than a lapse:
 * it adds the server and lets the user sign in on its own page, which is the only thing that works
 * for a server behind an identity provider.
 */
export const AddServer = ({ onAdded }: Props) => {
  const [address, setAddress] = useState('');
  const [identity, setIdentity] = useState('');
  const [password, setPassword] = useState('');
  /**
   * Whether Shiver may keep the password for this server.
   *
   * Off by default, and worth understanding either way. Sharkord signs a session for seven days and
   * has no way to refresh one, so Shiver's own connection to a server dies weekly: with no password
   * it goes quiet until the server is opened again, and with one it signs itself back in. The cost
   * is a second secret at rest, encrypted under a key the Android Keystore holds and Shiver cannot
   * extract — the same protection the session itself already gets.
   */
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** what the server said about itself, once checked; null until then */
  const [preview, setPreview] = useState<ServerInfo | null>(null);

  const check = useCallback(async () => {
    if (!address.trim() || busy) return;

    setBusy(true);
    setError(null);

    try {
      setPreview(await api.probeServer(address.trim()));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [address, busy]);

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
