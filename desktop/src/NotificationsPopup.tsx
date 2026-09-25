import { useCallback, useEffect, useState } from 'react';

import { api } from './api';
import { EVENTS, useCoreEvent } from './events';
import { NotificationList } from './components/NotificationList';
import { applyTheme } from '../../shared/web/theme';
import type { Notification } from './types';

/** The unified notification feed, in its own webview under the bell. */
export const NotificationsPopup = () => {
  const [notifications, setNotifications] = useState<Notification[]>([]);

  const refresh = useCallback(async () => setNotifications(await api.listNotifications()), []);
  const loadTheme = useCallback(async () => applyTheme(await api.getSettings()), []);

  useEffect(() => {
    void refresh();
    void loadTheme();
  }, [refresh, loadTheme]);

  useCoreEvent(EVENTS.feed, () => void refresh());
  useCoreEvent(EVENTS.settings, () => void loadTheme());

  const handleClear = useCallback(async () => {
    await api.clearNotifications();
    await refresh();
  }, [refresh]);

  const handleClose = useCallback(() => {
    api.closePopup().catch(() => undefined);
  }, []);

  /** Closes on Escape or on losing focus (a click in another webview is only visible as blur). */
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
