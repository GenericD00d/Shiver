import { api } from '../api';
import { SignInForm } from '../../../shared/web/components/SignInForm';
import type { ServerEntry } from '../types';

type Props = {
  server: ServerEntry;
  /** Shiver keeps its password already */
  remembered: boolean;
  /** signed in */
  onDone: () => void;
  onCancel: () => void;
};

/** Signing a server in again, for Shiver's own connection to it (its page keeps its own sign-in). */
export const ServerSignIn = ({ server, remembered, onDone, onCancel }: Props) => (
  <div className="modal-backdrop">
    <SignInForm
      key={server.id}
      serverName={server.name}
      identity={server.identity}
      hint={
        <>
          Sharkord expires a session after seven days and offers no way to renew one, so Shiver's watch of{' '}
          {server.origin.replace(/^https?:\/\//, '')} ends with it. Signing in gives Shiver a fresh session.
        </>
      }
      rememberHint={
        <>
          Shiver keeps the password and signs in again by itself, so this server never goes quiet. Stored encrypted
          under a key the phone's Keystore holds and Shiver cannot read out. Left unticked, you sign in here again
          when the session next expires.
        </>
      }
      remembered={remembered}
      onSubmit={async (identity, password, remember) => {
        await api.signInServer(server.id, identity, password, remember);
        onDone();
      }}
      onCancel={onCancel}
    />
  </div>
);
