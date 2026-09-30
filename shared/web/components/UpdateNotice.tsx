import { useCallback, useState } from 'react';

import { errorMessage } from '../errors';
import './ui.css';

type Props = {
  /** the newer version on offer; nothing is drawn without one */
  version: string | null;
  /** what taking it is called here ("Install" on desktop, "Get it" where Shiver cannot install itself) */
  takeLabel: string;
  /** shown on the button while `onTake` runs, if it takes a while */
  busyLabel?: string;
  /** a failure is shown under the version; on success the caller moves on (or the process ends) */
  onTake: () => Promise<void>;
  /** hides the bar for now; the offer stays where else it is kept */
  onLater: () => void;
  /** turns this version down for good */
  onSkip: (version: string) => Promise<void>;
};

/** A bar announcing a newer release: take it, Later, or Skip this version (remembered). */
export const UpdateNotice = ({ version, takeLabel, busyLabel, onTake, onLater, onSkip }: Props) => {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleTake = useCallback(async () => {
    setBusy(true);
    setError(null);

    try {
      await onTake();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }, [onTake]);

  const handleSkip = useCallback(async () => {
    if (!version) return;

    try {
      await onSkip(version);
    } catch {
      // a floor that could not be written is not worth a message; the bar going away is the answer
    }

    onLater();
  }, [onLater, onSkip, version]);

  if (!version) return null;

  return (
    <div className="update-notice">
      <span className="update-text">
        <strong>Shiver {version} is available.</strong>
        {error ? <small className="update-error">{error}</small> : null}
      </span>

      <button type="button" className="primary small" onClick={() => void handleTake()} disabled={busy}>
        {busy && busyLabel ? busyLabel : takeLabel}
      </button>
      <button type="button" className="ghost small" onClick={onLater}>
        Later
      </button>
      <button type="button" className="ghost small" onClick={() => void handleSkip()}>
        Skip this version
      </button>
    </div>
  );
};
