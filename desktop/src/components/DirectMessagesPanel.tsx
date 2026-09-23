import { useMemo, useState } from 'react';

import { relativeTime } from './NotificationList';
import type { DmEntry } from '../types';

type Props = {
  dms: DmEntry[];
  onOpen: (entryId: string, name: string, channelId: number) => void;
  onClose: () => void;
  /** the conversation currently rendered beside the list by its own server */
  openedKey: string | null;
  error: string | null;
};

/** The unified DM inbox, shaped like Sharkord's own DM list, plus which account each row belongs to. */
export const DirectMessagesPanel = ({ dms, onOpen, onClose, openedKey, error }: Props) => {
  const [query, setQuery] = useState('');

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();

    if (!needle) return dms;

    return dms.filter(
      (dm) =>
        dm.channel.name.toLowerCase().includes(needle) ||
        dm.serverName.toLowerCase().includes(needle)
    );
  }, [dms, query]);

  return (
    <div className="dm-layout">
      <div className="dm-sidebar">
        <div className="dm-sidebar-header">
          <span>Direct messages</span>
          <button type="button" className="dm-close" title="Close" onClick={onClose}>
            ✕
          </button>
        </div>

        <input
          className="dm-search"
          type="text"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search"
          spellCheck={false}
        />

        {error ? <p className="dm-error">{error}</p> : null}

        {filtered.length === 0 ? (
          <p className="dm-empty">
            {dms.length === 0 ? 'No conversations yet.' : 'Nothing matches that.'}
          </p>
        ) : null}

        <div className="dm-items">
          {filtered.map((dm) => (
            <button
              key={`${dm.entryId}:${dm.channel.channelId}`}
              type="button"
              className={
                openedKey === `${dm.entryId}:${dm.channel.channelId}`
                  ? 'dm-item selected'
                  : 'dm-item'
              }
              onClick={() => onOpen(dm.entryId, dm.channel.name, dm.channel.channelId)}
            >
              {dm.channel.iconUrl ? (
                <img src={dm.channel.iconUrl} alt="" />
              ) : (
                <span className="dm-avatar">{dm.channel.name.slice(0, 1).toUpperCase()}</span>
              )}

              <span className="dm-item-body">
                <span className="dm-item-name">{dm.channel.name}</span>
                <span className="dm-item-meta">
                  {dm.accountLabel ? `${dm.accountLabel} · ` : ''}
                  {dm.serverName}
                </span>
              </span>

              {dm.channel.lastMessageAt ? (
                <span className="dm-item-time">{relativeTime(dm.channel.lastMessageAt)}</span>
              ) : null}
            </button>
          ))}
        </div>
      </div>

      {/* empty while a conversation is open: the server's own webview covers this region */}
      {openedKey ? null : (
        <div className="dm-hint">
          <p>Pick a conversation to read it here.</p>
        </div>
      )}
    </div>
  );
};
