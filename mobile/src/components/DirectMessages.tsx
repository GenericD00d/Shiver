import { useEffect, useState } from 'react';

import { api } from '../api';
import type { DmEntry } from '../types';

type Props = {
  /** opens the server the conversation is on, landing on that conversation */
  onOpen: (entryId: string, userName: string) => void;
};

const initial = (name: string) => name.trim()[0]?.toUpperCase() ?? '?';

/**
 * How long ago, in as few characters as fit beside a server name.
 *
 * Shown because the list is *ordered* by it: an order the reader cannot see the reason for looks
 * arbitrary, and this is what makes it obviously not. Coarse on purpose — the exact minute of a
 * conversation from March is not why anyone is looking at this screen.
 */
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
 * Every server's conversations, on Shiver's own screen.
 *
 * This list used to be drawn inside whichever server's page was on screen, because Android gives a
 * window one webview and the rail has to live somewhere. That handed one server the display name of
 * everyone the user privately messages on every *other* server they had added. A page on a server's
 * origin has no Tauri IPC, so moving the list here is what makes it unreadable to that page — there
 * is no way to hide data from a page that has to render it.
 *
 * Ordered by when the last message arrived and by nothing else — see `collect_dms`. It used to be
 * grouped by server in rail order, which meant the list reshuffled every time the user switched
 * servers, so the same conversation was never twice in the same place. Which server a conversation
 * is on is written on the row; that is not a reason to sort by it.
 *
 * The server currently on screen appears from whatever Shiver last knew of it, since the core drops
 * its socket to that one — its own page reports for itself. So its timestamps here can lag by a
 * visit, while the panel inside that page is live.
 */
export const DirectMessages = ({ onOpen }: Props) => {
  const [dms, setDms] = useState<DmEntry[] | null>(null);

  useEffect(() => {
    api
      .listDms()
      .then(setDms)
      .catch(() => setDms([]));
  }, []);

  if (dms === null) return <p className="hint">Looking…</p>;

  if (!dms.length) {
    return (
      <p className="hint">
        No conversations on your other servers yet. Shiver finds them on the servers it is watching,
        which is every server but the one you are looking at.
      </p>
    );
  }

  return (
    <ul className="servers">
      {dms.map((dm) => (
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
  );
};
