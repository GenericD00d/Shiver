import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';

type Props = {
  onAdded: () => void;
};

/**
 * Adds a server, signing in on the way.
 *
 * One step. It used to look the server up and then add it, which made the user press a button whose
 * only job was to prove the address worked — the address is checked as part of adding either way,
 * and a server that does not answer still fails.
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
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const add = useCallback(async () => {
    if (!address.trim() || busy) return;

    setBusy(true);
    setError(null);

    try {
      await api.addServer(
        address.trim(),
        identity.trim() || null,
        password || null,
        remember && !!password
      );
      onAdded();
    } catch (cause) {
      setError(errorMessage(cause));
      setBusy(false);
    }
  }, [address, busy, identity, onAdded, password, remember]);

  return (
    <form
      className="panel"
      onSubmit={(event) => {
        event.preventDefault();
        void add();
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
          onChange={(event) => setAddress(event.target.value)}
        />
      </label>

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


      <label className="checkbox">
        <input
          type="checkbox"
          checked={remember}
          onChange={(event) => setRemember(event.target.checked)}
        />
        <span>
          Stay signed in on this device
          <small>
            Lets Shiver keep watching this server for messages after its session expires, which
            Sharkord makes it do every seven days. Without this, the server goes quiet until you
            open it again. Your password is stored encrypted under a key the phone's Keystore holds.
          </small>
        </span>
      </label>

      <p className="hint">
        Shiver signs in for you, so the server opens straight into the app. Your password goes only to
        this server, and the session it returns is kept in Android's encrypted store. Leave both
        empty to sign in on the server's own page instead.
      </p>

      {error ? <p className="error">{error}</p> : null}

      <button type="submit" className="primary wide" disabled={busy || !address.trim()}>
        {busy ? 'Adding…' : 'Add server'}
      </button>
    </form>
  );
};
