import type { ServerEntry } from '../types';

type Props = {
  servers: ServerEntry[];
  onOpen: (id: string) => void;
  onRemove: (id: string) => void;
  onAdd: () => void;
  /** servers whose session Shiver could not renew, which are waiting to be signed in */
  signedOut: string[];
  /** servers Shiver cannot watch at all, keyed by entry id */
  problems: Record<string, string>;
  onAcceptAnySize: (id: string, accept: boolean) => void;
  /** servers Shiver can sign in again by itself, because it was asked to keep the password */
  remembered: string[];
  onSignIn: (id: string) => void;
};

const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase() ?? '')
    .join('') || '?';

/**
 * Managing the list of servers.
 *
 * Not a switcher any more — the rail is that, and Shiver opens the last server used rather than
 * showing a front page. This lives under settings, which is where the things you do rarely go.
 */
export const ServerList = ({
  servers,
  onOpen,
  onRemove,
  onAdd,
  signedOut,
  problems,
  remembered,
  onSignIn,
  onAcceptAnySize
}: Props) => (
  <>
    <ul className="servers">
      {servers.map((server) => (
        <li key={server.id} className={problems[server.id] ? "has-problem" : undefined}>
          <button type="button" className="server" onClick={() => onOpen(server.id)}>
            {server.iconUrl ? (
              <img src={server.iconUrl} alt="" />
            ) : (
              <span className="server-icon">{initials(server.name)}</span>
            )}

            <span className="server-text">
              <span className="server-name">{server.name}</span>
              <span className="server-origin">
                {signedOut.includes(server.id)
                  ? 'Session expired — sign in to keep watching'
                  : remembered.includes(server.id)
                    ? `${server.origin.replace(/^https?:\/\//, '')} · stays signed in`
                    : server.origin.replace(/^https?:\/\//, '')}
              </span>

              {/* The limit is Shiver being careful with servers it knows nothing about. Whether
                  this particular one deserves that is something only the person who knows whose
                  server it is can say, so it is said here, once, on the row it is about. */}
              {server.acceptAnySize ? (
                <span className="server-note">Any message size allowed</span>
              ) : null}
            </span>
          </button>

          <button
            type="button"
            className={signedOut.includes(server.id) ? 'primary small' : 'ghost small'}
            aria-label={`Sign in to ${server.name}`}
            onClick={() => onSignIn(server.id)}
          >
            Sign in
          </button>

          {problems[server.id] || server.acceptAnySize ? (
            <button
              type="button"
              className="ghost small"
              aria-label={
                server.acceptAnySize
                  ? `Stop accepting any message size from ${server.name}`
                  : `Accept any message size from ${server.name}`
              }
              onClick={() => onAcceptAnySize(server.id, !server.acceptAnySize)}
            >
              {server.acceptAnySize ? 'Limit' : 'Trust'}
            </button>
          ) : null}

          <button
            type="button"
            className="ghost small"
            aria-label={`Remove ${server.name}`}
            onClick={() => onRemove(server.id)}
          >
            Remove
          </button>

          {/* On its own line under the row rather than inside the name column, which is as wide as
              whatever the buttons leave — a sentence squeezed into eight characters is not a
              warning anyone reads. Said here as well as on the shade, because a notification is
              gone by the time anyone wonders why a server went quiet. */}
          {problems[server.id] ? (
            <span className="server-problem">{problems[server.id]}</span>
          ) : null}
        </li>
      ))}
    </ul>

    <button type="button" className="primary wide" onClick={onAdd}>
      Add a server
    </button>
  </>
);
