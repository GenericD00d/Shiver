import { useCallback, useEffect, useState } from 'react';

import { api, errorMessage } from '../api';
import { EVENTS, useCoreEvent } from '../events';

/**
 * A bar announcing a newer release on launch. Install now, Later (the bell keeps the offer) or Skip
 * this version (remembered).
 */
export const UpdateNotice = () => {
  const [version, setVersion] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .availableUpdate()
      .then(setVersion)
      .catch(() => undefined);
  }, []);

  // found after launch, by the core's own check; "Later" holds until a newer one is found
  useCoreEvent<string>(EVENTS.update, setVersion);

  const handleInstall = useCallback(async () => {
    setBusy('Downloading…');

    try {
      await api.installUpdate();
    } catch (cause) {
      // on success the installer takes over and this process ends, so only a failure comes back
      setBusy(null);
      setError(errorMessage(cause));
    }
  }, []);

  const handleSkip = useCallback(async () => {
    if (!version) return;

    try {
      await api.skipUpdate(version);
    } catch {
      // a floor that could not be written is not worth a message; the bar going away is the answer
    }

    setVersion(null);
  }, [version]);

  if (!version) return null;

  return (
    <div className="update-notice">
      <span className="update-text">
        <strong>Shiver {version} is available.</strong>
        {error ? <small className="update-error">{error}</small> : null}
      </span>

      <button type="button" className="primary small" onClick={handleInstall} disabled={busy !== null}>
        {busy ?? 'Install'}
      </button>
      <button type="button" className="ghost small" onClick={() => setVersion(null)}>
        Later
      </button>
      <button type="button" className="ghost small" onClick={handleSkip}>
        Skip this version
      </button>
    </div>
  );
};
