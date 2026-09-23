import type { ServerEntry } from '../types';

/** An action a server page's rail asked for by navigating here; confirmed on Shiver's own page, since any page can navigate. */
export type ConfirmAction = 'remove' | 'forgetpw' | 'logout';

export type BootState =
  | { kind: 'waiting' }
  | { kind: 'connecting'; server: ServerEntry }
  | { kind: 'failed'; server: ServerEntry }
  | { kind: 'confirm'; action: ConfirmAction; server: ServerEntry }
  | { kind: 'empty' };

type Props = {
  state: BootState;
  onRetry: (server: ServerEntry) => void;
  onAdd: () => void;
  onConfirm: (confirmed: boolean) => void;
};

const QUESTIONS: Record<ConfirmAction, { question: (name: string) => string; action: string }> = {
  remove: { question: (name) => `Remove ${name} from Shiver?`, action: 'Remove' },
  forgetpw: { question: (name) => `Forget the saved password for ${name}?`, action: 'Forget' },
  logout: { question: (name) => `Log out of ${name}?`, action: 'Log out' }
};

/** Shiver's only front page (mobile has no home screen): a spinner, a failure, a confirmation or "add a server". */
export const Boot = ({ state, onRetry, onAdd, onConfirm }: Props) => {
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

  if (state.kind === 'confirm') {
    const { question, action } = QUESTIONS[state.action];

    return (
      <div className="boot">
        <p>{question(state.server.name)}</p>

        <button type="button" className="primary" onClick={() => onConfirm(true)}>
          {action}
        </button>

        <button type="button" className="ghost" onClick={() => onConfirm(false)}>
          Cancel
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
