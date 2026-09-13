import type { ServerEntry } from '../types';

type State =
  | { kind: 'waiting' }
  | { kind: 'connecting'; server: ServerEntry }
  | { kind: 'failed'; server: ServerEntry }
  | { kind: 'empty' };

type Props = {
  state: State;
  onRetry: (server: ServerEntry) => void;
  onAdd: () => void;
};

/**
 * What Shiver shows while it is on its way somewhere else.
 *
 * The mobile client has no home screen: it opens the server you were last in. So this is the whole
 * of Shiver's own front page, and it is only ever transient — a spinner until the server answers, or
 * the reason it did not. The same two states the desktop client shows for a server that is coming
 * up, said the same way.
 */
export const Boot = ({ state, onRetry, onAdd }: Props) => {
  if (state.kind === 'empty') {
    return (
      <div className="boot">
        <p className="empty">No servers yet. Add one to get started.</p>

        <button type="button" className="primary" onClick={onAdd}>
          Add a server
        </button>
      </div>
    );
  }

  if (state.kind === 'failed') {
    return (
      <div className="boot">
        <p className="boot-failed">Failed to connect to {state.server.name}</p>

        <button type="button" className="primary" onClick={() => onRetry(state.server)}>
          Try again
        </button>
      </div>
    );
  }

  return (
    <div className="boot">
      <span className="spinner" aria-hidden="true" />

      <p className="hint" role="status">
        {state.kind === 'connecting' ? `Connecting to ${state.server.name}…` : 'Starting Shiver…'}
      </p>
    </div>
  );
};
