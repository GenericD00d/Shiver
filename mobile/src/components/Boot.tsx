import { useEffect, useState } from 'react';

import type { ServerEntry } from '../types';

/** A rail menu action that is confirmed before it runs. */
export type ConfirmAction = 'remove' | 'forgetpw' | 'logout';

export type BootState =
  | { kind: 'waiting' }
  /** `until`: when the core will open it, if it is waiting for the server to take another join */
  | { kind: 'connecting'; server: ServerEntry; until?: number }
  | { kind: 'failed'; server: ServerEntry }
  | { kind: 'confirm'; action: ConfirmAction; server: ServerEntry }
  /** Shiver's own page, asked for from the quick rail; `server` is the one it came from */
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

  // Shiver's own page, asked for from the quick rail: the rail here has the rest, and this goes back
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

      {state.kind === 'connecting' && state.until ? <JoinWait until={state.until} /> : null}
    </div>
  );
};

/** Counts down a wait for the server to take another join, and says why there is one. */
const JoinWait = ({ until }: { until: number }) => {
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);

    return () => window.clearInterval(timer);
  }, []);

  const seconds = Math.max(0, Math.ceil((until - now) / 1000));

  return (
    <p className="hint">
      {seconds > 0 ? `In ${seconds} s: ` : ''}Sharkord lets an account connect only a few times a minute, so
      Shiver waits rather than be turned away.
    </p>
  );
};
