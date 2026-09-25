import { useEffect, useState } from 'react';

import type { ServerEntry } from '../types';

/** A rail menu action that is confirmed before it runs. */
export type ConfirmAction = 'remove' | 'forgetpw' | 'logout';

export type BootState =
  | { kind: 'waiting' }
  | { kind: 'connecting'; server: ServerEntry }
  | { kind: 'failed'; server: ServerEntry }
  | { kind: 'confirm'; action: ConfirmAction; server: ServerEntry }
  /** a server's page came back to the rail; `server` is the one it left */
  | { kind: 'home'; server: ServerEntry }
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

/** Shiver's front page: a spinner, a failure, a confirmation, the way back from the rail (the page left, tapped), or "add a server". */
const ARM_MS = 1000;

export const Boot = ({ state, onRetry, onAdd, onConfirm }: Props) => {
  const [armed, setArmed] = useState(false);

  useEffect(() => {
    setArmed(false);

    const timer = window.setTimeout(() => setArmed(true), ARM_MS);

    return () => window.clearTimeout(timer);
  }, [state]);

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

  // the still of the page just left shows behind the rail; tapping it goes back, as for a drawer
  if (state.kind === 'home') {
    return (
      <button
        type="button"
        className="boot-return"
        aria-label={`Back to ${state.server.name}`}
        onClick={() => onRetry(state.server)}
      />
    );
  }

  if (state.kind === 'confirm') {
    const { question, action } = QUESTIONS[state.action];

    return (
      <div className="boot">
        <p>{question(state.server.name)}</p>

        <button type="button" className="primary" disabled={!armed} onClick={() => onConfirm(true)}>
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
