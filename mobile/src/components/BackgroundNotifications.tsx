import { useCallback, useEffect, useState } from 'react';

import { api, errorMessage } from '../api';
import type { PushStatus } from '../types';

/** Distributors people actually have, so the list reads as names rather than package ids. */
const KNOWN: Record<string, string> = {
  'io.heckel.ntfy': 'ntfy',
  'org.unifiedpush.distributor.nextpush': 'NextPush',
  'org.unifiedpush.distributor.fcm': 'UnifiedPush via FCM',
  'com.sunup.distributor': 'Sunup'
};

const label = (packageName: string) => KNOWN[packageName] ?? packageName;

/** What each server's row says under its name. The failure is the one worth explaining. */
const SERVER_STATE: Record<string, string> = {
  off: 'Will not wake this phone',
  waiting: 'Waiting for the distributor to answer…',
  ready: 'Ready — this server can wake you',
  failed: 'The distributor refused. Open it, check it is set up, then turn this off and on again'
};

/**
 * UnifiedPush settings: a distributor app holds one connection for every app and wakes Shiver
 * while it is closed. Needs a distributor installed and the companion plugin on the server; without
 * either, Shiver is simply quiet while closed.
 */
export const BackgroundNotifications = () => {
  const [status, setStatus] = useState<PushStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const read = useCallback(() => {
    api
      .pushStatus()
      .then(setStatus)
      .catch(() => setStatus(null));
  }, []);

  useEffect(read, [read]);

  // an endpoint arrives as a broadcast some moments after the distributor is picked, so the screen
  // is told rather than left showing "waiting" until something else redraws it
  useEffect(() => {
    const stop = api.onPush(read);

    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [read]);

  const chooseServer = useCallback(
    async (entryId: string, wanted: boolean) => {
      setError(null);

      try {
        await api.setPushServer(entryId, wanted);
        read();
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    [read]
  );

  const choose = useCallback(
    async (distributor: string) => {
      setBusy(true);
      setError(null);

      try {
        await api.setPushDistributor(distributor);
        read();
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setBusy(false);
      }
    },
    [read]
  );

  if (!status) return null;

  if (!status.distributors.length) {
    return (
      <>
        <p className="hint">
          To be told about messages after Android has closed Shiver, install a UnifiedPush
          distributor — <strong>ntfy</strong> is the usual one.
        </p>
        <p className="hint">
          The server needs the Shiver plugin too. Without either, Shiver simply stays quiet while
          it is closed.
        </p>
      </>
    );
  }

  return (
    <>
      <p className="hint">
        One connection for every app on your phone, so Shiver can be told about messages without
        running. The server needs the Shiver plugin to use it.
      </p>

      {error ? <p className="error">{error}</p> : null}

      <ul className="servers">
        {status.distributors.map((distributor) => (
          <li key={distributor}>
            <button
              type="button"
              className="server"
              disabled={busy}
              onClick={() => void choose(distributor)}
            >
              <span className="server-icon">{label(distributor).slice(0, 1).toUpperCase()}</span>

              <span className="server-text">
                <span className="server-name">{label(distributor)}</span>
                <span className="server-origin">
                  {status.chosen === distributor ? 'In use' : 'Tap to use this one'}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ul>

      {status.chosen ? (
        <>
          <h3 className="section">Which servers may wake you</h3>

          <p className="hint">
            One at a time: each is a separate address this phone can be reached on. Nothing is
            registered until you say so.
          </p>

          {status.servers.length === 0 ? (
            <p className="hint">No servers yet. Add one and it will appear here.</p>
          ) : null}

          {status.servers.map((server) => (
            <label className="checkbox" key={server.id}>
              <input
                type="checkbox"
                checked={server.wanted}
                onChange={(event) => void chooseServer(server.id, event.target.checked)}
              />
              <span>
                {server.name}
                <small>{SERVER_STATE[server.state]}</small>
              </span>
            </label>
          ))}
        </>
      ) : null}
    </>
  );
};
