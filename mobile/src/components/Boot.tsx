import { Confirm } from '../../../shared/web/components/Confirm';
import { Connecting } from '../../../shared/web/components/Connecting';
import type { ServerEntry } from '../types';

/** A rail menu action that is confirmed before it runs. */
export type ConfirmAction = 'remove' | 'forgetpw' | 'logout';

export type BootState =
  | { kind: 'waiting' }
  /** `until`: when the core will open it, if it is waiting for the server to take another join */
  | { kind: 'connecting'; server: ServerEntry; until?: number }
  | { kind: 'failed'; server: ServerEntry }
  | { kind: 'confirm'; action: ConfirmAction; server: ServerEntry }
  /** Shiver's own page after a swipe for the rail that could not be drawn natively; `server` is the one it came from */
  | { kind: 'home'; server: ServerEntry }
  | { kind: 'empty' };

type Props = {
  state: BootState;
  onRetry: (server: ServerEntry) => void;
  onAdd: () => void;
  onConfirm: (confirmed: boolean) => void;
};

const QUESTIONS: Record<ConfirmAction, { question: (name: string) => string; hint?: string; action: string }> = {
  remove: {
    question: (name) => `Remove ${name}?`,
    hint: 'Shiver forgets its sign-in, saved password and page storage. Nothing changes on the server.',
    action: 'Remove'
  },
  forgetpw: { question: (name) => `Forget the saved password for ${name}?`, action: 'Forget' },
  logout: { question: (name) => `Log out of ${name}?`, action: 'Log out' }
};

/** The menu item that asked sits where the confirming button now is: a touch must not answer both. */
const ARM_MS = 1000;

/** Shiver's front page: a spinner, a failure, a confirmation, the way back to the server left, or "add a server". */
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
    return <Connecting serverName={state.server.name} failed onRetry={() => onRetry(state.server)} />;
  }

  // the quick rail could not be drawn, so the swipe landed here: the rail beside this has the rest, and this goes back
  if (state.kind === 'home') {
    return (
      <div className="boot">
        <button type="button" className="primary" onClick={() => onRetry(state.server)}>
          Back to {state.server.name}
        </button>
      </div>
    );
  }

  if (state.kind === 'confirm') {
    const { question, hint, action } = QUESTIONS[state.action];

    return (
      <div className="modal-backdrop">
        <Confirm
          question={question(state.server.name)}
          hint={hint}
          action={action}
          armMs={ARM_MS}
          onConfirm={() => onConfirm(true)}
          onCancel={() => onConfirm(false)}
        />
      </div>
    );
  }

  return (
    <Connecting
      serverName={state.kind === 'connecting' ? state.server.name : null}
      until={state.kind === 'connecting' ? state.until : undefined}
    />
  );
};
