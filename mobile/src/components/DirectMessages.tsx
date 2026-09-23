import { useEffect, useMemo, useState } from 'react';

import { api } from '../api';
import type { DmEntry } from '../types';

type Props = {
  /** opens the server the conversation is on, landing on that conversation */
  onOpen: (entryId: string, userName: string) => void;
};

const initial = (name: string) => name.trim()[0]?.toUpperCase() ?? '?';

/** A coarse "how long ago", shown because the list is ordered by it. */
const when = (at: number | null) => {
  if (at === null) return '';

  const seconds = Math.max(0, (Date.now() - at) / 1000);

  if (seconds < 60) return 'now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`;
  if (seconds < 604800) return `${Math.floor(seconds / 86400)}d`;

  return new Date(at).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
};

/**
 * Every server's conversations, newest first (`collect_dms`). Drawn here rather than in a server's
 * page so no server learns who the user talks to elsewhere.
 */
export const DirectMessages = ({ onOpen }: Props) => {
  const [dms, setDms] = useState<DmEntry[] | null>(null);
  const [query, setQuery] = useState('');

  /** Reloaded whenever the core publishes, since servers connect after this screen mounts. */
  useEffect(() => {
    let live = true;

    const load = () => {
      api
        .listDms()
        .then((next) => {
          if (live) setDms(next);
        })
        .catch(() => {
          if (live) setDms([]);
        });
    };

    load();

    const pending = api.onUnread(load);

    return () => {
      live = false;
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, []);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();

    if (!needle) return dms ?? [];

    // name or server, the same two desktop matches on: those are the two things on a row, and
    // searching what is not shown is how a search box gets a reputation for being broken
    return (dms ?? []).filter(
      (dm) =>
        dm.userName.toLowerCase().includes(needle) ||
        dm.serverName.toLowerCase().includes(needle)
    );
  }, [dms, query]);

  if (dms === null) return <p className="hint">Looking…</p>;

  if (!dms.length) {
    return (
      <p className="hint">
        No conversations yet. Every server you are signed in to is listed here together, so this is
        all of them rather than one server's.
      </p>
    );
  }

  return (
    <>
      <input
        className="dm-search"
        type="text"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Search"
        spellCheck={false}
        autoComplete="off"
      />

      {filtered.length === 0 ? (
        <p className="hint">Nobody by that name.</p>
      ) : null}

      <ul className="servers">
        {filtered.map((dm) => (
          <li key={`${dm.entryId}:${dm.channelId}`}>
            <button
              type="button"
              className="server"
              onClick={() => onOpen(dm.entryId, dm.userName)}
            >
              <span className="server-icon">{initial(dm.userName)}</span>

              <span className="server-text">
                <span className="server-name">{dm.userName}</span>
                <span className="server-origin">
                  {dm.accountLabel ? `${dm.serverName} · as ${dm.accountLabel}` : dm.serverName}
                </span>
              </span>

              {/* what the list is sorted by, said out loud */}
              <span className="dm-when">{when(dm.lastMessageAt)}</span>
            </button>
          </li>
        ))}
      </ul>
    </>
  );
};
