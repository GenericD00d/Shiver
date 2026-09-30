import { type ReactNode, useEffect, useState } from 'react';

import './ui.css';

type Props = {
  question: string;
  /** what answering yes does, when the question alone does not say */
  hint?: ReactNode;
  /** the confirming button's label: what it does, never "OK" */
  action: string;
  onConfirm: () => void;
  onCancel: () => void;
  /**
   * How long the confirming button stays disabled, so the touch that opened this (a menu item where
   * the button now is) cannot also answer it. 0 arms it at once, and focuses it.
   */
  armMs?: number;
};

/** Asks before something that cannot be taken back, as a dialog card. */
export const Confirm = ({ question, hint, action, onConfirm, onCancel, armMs = 0 }: Props) => {
  const [armed, setArmed] = useState(armMs === 0);

  useEffect(() => {
    if (armMs === 0) return;

    setArmed(false);

    const timer = window.setTimeout(() => setArmed(true), armMs);

    return () => window.clearTimeout(timer);
  }, [armMs, question]);

  return (
    <div className="card" role="alertdialog" aria-label={question}>
      <h1>{question}</h1>
      {hint ? <p className="hint">{hint}</p> : null}

      <div className="actions">
        <button type="button" className="ghost" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="primary" onClick={onConfirm} disabled={!armed} autoFocus={armMs === 0}>
          {action}
        </button>
      </div>
    </div>
  );
};
