import { useCallback, useEffect, useState } from 'react';

import { api, errorMessage } from '../api';

/**
 * Says a newer Shiver exists, once, where it will actually be seen.
 *
 * The bell already carries an entry for this, and the bell is where somebody looks when they want
 * to know what they have missed — not where they look to find out that the thing they are using is
 * out of date. So this sits over the top of the window on launch instead.
 *
 * **A bar rather than a dialog.** Nothing here is urgent enough to stop somebody reading their
 * messages, and a modal on launch is the kind of thing people learn to dismiss without reading.
 * It can be ignored entirely and the bell keeps the offer.
 *
 * Three ways out, and they mean different things. **Install** takes it now. **Later** puts the bar
 * away for this run and the offer stays in the bell. **Skip this version** means "stop telling me
 * about this one" — persisted, so it survives a restart — and a release after it is news again.
 * Being asked twice about the same version is how a prompt teaches people to ignore prompts.
 */
export const UpdateNotice = () => {
  const [version, setVersion] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;

    // The check runs on a timer of its own after launch, so the answer is usually not ready the
    // first time this asks. Polling briefly is what turns "there is an update" into something seen
    // on the launch it was found, rather than on the next one.
    const ask = () => {
      api
        .availableUpdate()
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
