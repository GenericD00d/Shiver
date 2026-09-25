import { useCallback, useEffect, useState } from 'react';

import { api } from '../api';

/**
 * Announces a newer release on Shiver's screens. Shiver cannot install packages itself (that would
 * need `REQUEST_INSTALL_PACKAGES`), so "Get it" opens the releases page; "Skip" is remembered.
 */
export const UpdateNotice = () => {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    api
      .updateAvailable()
      .then(setVersion)
      .catch(() => undefined);

    // found after launch, by the core's own check
    const stop = api.onUpdate(setVersion);

    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, []);

  const handleSkip = useCallback(async () => {
    if (!version) return;

    try {
      await api.skipUpdate(version);
    } catch {
      // not worth a message; the notice going away is the answer
    }

    setVersion(null);
  }, [version]);

  if (!version) return null;

  return (
    <div className="update-notice">
      <strong>Shiver {version} is available.</strong>

      <div className="update-actions">
        <button
          type="button"
          className="primary small"
          onClick={() => void api.openReleases().catch(() => undefined)}
        >
          Get it
        </button>
        <button type="button" className="ghost small" onClick={() => setVersion(null)}>
          Later
        </button>
        <button type="button" className="ghost small" onClick={handleSkip}>
          Skip this version
        </button>
      </div>
    </div>
  );
};
