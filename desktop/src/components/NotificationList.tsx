import { useCallback, useState } from 'react';

import { api, errorMessage } from '../api';
import type { Notification } from '../types';

type Props = {
  notifications: Notification[];
  onChanged: () => void;
  /** take the user to where the message is. absent where there is nowhere to go from. */
  onOpen?: (entry: Notification) => void;
};

export const relativeTime = (at: number) => {
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));

  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;

  return `${Math.floor(seconds / 86400)}d ago`;
};

/** Shared by the bell popup and anywhere else the feed is listed. */
export const NotificationList = ({ notifications, onChanged, onOpen }: Props) => {
  const handleMute = useCallback(
    async (entryId: string, channelId: number) => {
      await api.setChannelMuted(entryId, channelId, true);

      onChanged();
    },
    [onChanged]
  );

  /**
   * What the install button is doing, so it can say so.
   *
   * On success nothing is ever set: the installer takes over and Shiver exits mid-click. Only a
   * failure comes back, and it is put where the button was rather than swallowed — a button that
   * does nothing is how this feature would earn its distrust.
   */
  const [installing, setInstalling] = useState<string | null>(null);

  const handleInstall = useCallback(async () => {
    setInstalling('Downloading…');

    try {
      await api.installUpdate();
    } catch (error) {
      setInstalling(errorMessage(error));
    }
  }, []);

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
        <li
          key={entry.id}
          className={`feed-item${entry.read ? ' read' : ''}${entry.update ? ' update' : ''}${
            onOpen && !entry.update ? ' openable' : ''
          }`}
          // The row itself, rather than a wrapping button: it already contains buttons, and a
          // button inside a button is not something html allows. The mute and install controls stop
          // the event themselves so clicking one does not also navigate.
          onClick={onOpen && !entry.update ? () => onOpen(entry) : undefined}
        >
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

          {entry.update ? (
            <button
              type="button"
              className="primary small"
              onClick={(event) => {
                event.stopPropagation();
                handleInstall();
              }}
              disabled={installing !== null}
            >
              {installing ?? 'Install'}
            </button>
          ) : null}

          {entry.channelId !== null && !entry.isDm ? (
            <button
              type="button"
              className="ghost small"
              title={`Mute #${entry.channelName ?? ''}`}
              onClick={(event) => {
                event.stopPropagation();
                handleMute(entry.entryId, entry.channelId as number);
              }}
            >
              Mute
            </button>
          ) : null}
        </li>
      ))}
    </ul>
  );
};
