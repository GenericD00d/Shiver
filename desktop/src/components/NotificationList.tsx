import { useCallback } from 'react';

import { api } from '../api';
import type { Notification } from '../types';

type Props = {
  notifications: Notification[];
  onChanged: () => void;
};

export const relativeTime = (at: number) => {
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));

  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;

  return `${Math.floor(seconds / 86400)}d ago`;
};

/** Shared by the bell popup and anywhere else the feed is listed. */
export const NotificationList = ({ notifications, onChanged }: Props) => {
  const handleMute = useCallback(
    async (entryId: string, channelId: number) => {
      await api.setChannelMuted(entryId, channelId, true);

      onChanged();
    },
    [onChanged]
  );

  if (notifications.length === 0) {
    return (
      <p className="empty">
        Nothing yet. Messages from every server you are signed in to land here.
      </p>
    );
  }

  return (
    <ul className="feed">
      {notifications.map((entry) => (
        <li key={entry.id} className={entry.read ? 'feed-item read' : 'feed-item'}>
          {entry.iconUrl ? (
            <img src={entry.iconUrl} alt="" />
          ) : (
            <span className="fallback">{entry.author.slice(0, 1).toUpperCase()}</span>
          )}

          <div className="feed-body">
            <div className="feed-meta">
              <strong>{entry.author}</strong>
              <span className="dim">
                {entry.isDm ? 'DM' : `#${entry.channelName ?? 'unknown'}`} · {entry.serverName}
              </span>
              <span className="dim">{relativeTime(entry.at)}</span>
            </div>
            <p>{entry.body}</p>
          </div>

          {entry.channelId !== null && !entry.isDm ? (
            <button
              type="button"
              className="ghost small"
              title={`Mute #${entry.channelName ?? ''}`}
              onClick={() => handleMute(entry.entryId, entry.channelId as number)}
            >
              Mute
            </button>
          ) : null}
        </li>
      ))}
    </ul>
  );
};
