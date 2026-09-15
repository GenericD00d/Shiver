import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useState } from 'react';

import { api } from './api';
import { NotificationList } from './components/NotificationList';
import { applyTheme } from './theme';
import type { Notification } from './types';

const FEED_EVENT = 'shiver://feed';
const SETTINGS_EVENT = 'shiver://settings';

/**
 * The unified notification feed, in its own webview anchored under the bell.
 *
 * Entirely separate from the bell: it is created when opened and closed when dismissed, so the bell
 * never has to change shape to accommodate it.
 */
export const NotificationsPopup = () => {
  const [notifications, setNotifications] = useState<Notification[]>([]);

  const refresh = useCallback(async () => {
    const [feed, settings] = await Promise.all([api.listNotifications(), api.getSettings()]);

    setNotifications(feed);
    applyTheme(settings);
  }, []);

  useEffect(() => {
    refresh();

    const pending = [
      listen(FEED_EVENT, () => refresh()),
      listen(SETTINGS_EVENT, () => refresh())
    ];

    return () => {
      for (const handle of pending) {
        handle.then((unsubscribe) => unsubscribe()).catch(() => undefined);
      }
    };
  }, [refresh]);

  const handleClear = useCallback(async () => {
    await api.clearNotifications();
    await refresh();
  }, [refresh]);

  const handleClose = useCallback(() => {
    api.closePopup().catch(() => undefined);
  }, []);

  /**
   * Dismisses the feed when the user clicks away from it, or presses Escape.
   *
   * A click outside lands in a different webview — a server's page, or the shell — and nothing in
   * one webview can see a click in another, so there is no outside-click handler to write. Losing
   * focus is the same event seen from this side, and it is the only signal this page gets.
   *
   * It fires for the window losing focus too, which is the behaviour a popover should have anyway.
   */
  useEffect(() => {
    const dismiss = () => api.dismissPopup().catch(() => undefined);

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') dismiss();
    };

    window.addEventListener('blur', dismiss);
    window.addEventListener('keydown', onKeyDown);

    return () => {
      window.removeEventListener('blur', dismiss);
      window.removeEventListener('keydown', onKeyDown);
    };
  }, []);

  return (
    <div className="popup">
      <div className="popup-header">
        <strong>Notifications</strong>
        <div className="popup-actions">
          <button
            type="button"
            className="ghost small"
            onClick={handleClear}
            disabled={notifications.length === 0}
          >
            Clear
          </button>
          <button type="button" className="ghost small" onClick={handleClose}>
            Close
          </button>
        </div>
      </div>

      <div className="popup-body">
        <NotificationList
          notifications={notifications}
          onChanged={refresh}
          onOpen={(entry) =>
            api
              .openMessage(entry.entryId, entry.channelId, entry.isDm, entry.author)
              .catch(() => undefined)
          }
        />
      </div>
    </div>
  );
};
