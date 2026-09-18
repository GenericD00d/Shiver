import { useCallback, useEffect, useState } from 'react';

import { api } from '../api';

/**
 * Says a newer Shiver exists, where it will be seen rather than found.
 *
 * The settings screen already names the newest version, and Android's shade gets a notification —
 * but a notification is easy to swipe past and settings is somewhere people go for a reason. This
 * sits on Shiver's own screens, which is where a launch lands.
 *
 * **Android installs its own packages.** Shiver does not: that would need
 * `REQUEST_INSTALL_PACKAGES`, a permission letting it install anything at all, which is a poor
 * trade for saving a tap. So the button opens the releases page and Android's installer takes it
 * from there, asking the browser for permission rather than Shiver.
 *
 * Three ways out, meaning different things. **Get it** opens the page. **Later** puts this away for
 * now. **Skip this version** means "stop telling me about this one" — persisted, so it survives a
 * restart — and a release after it is news again.
 */
export const UpdateNotice = () => {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let live = true;

    // the check runs on a timer of its own after launch, so the answer is usually not ready the
    // first time this asks
    const ask = () => {
      api
        .updateAvailable()
        .then((found) => {
          if (live && found) setVersion(found);
        })
        .catch(() => undefined);
    };

    ask();

    const timer = window.setInterval(ask, 10_000);

    return () => {
      live = false;
      window.clearInterval(timer);
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
