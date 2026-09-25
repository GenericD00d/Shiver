import { useCallback, useEffect, useRef, useState } from 'react';

import { api } from './api';
import { EVENTS, useCoreEvent } from './events';
import { BellIcon } from './components/icons';
import { playNotificationSound } from './sounds';
import { applyTheme } from '../../shared/web/theme';

/** The notification bell, in a fixed 48x48 webview; the feed opens in a separate webview below it. */
export const Bell = () => {
  const [unread, setUnread] = useState(0);
  const [open, setOpen] = useState(false);
  // the newest notification already sounded for, so a refresh never re-pings
  const lastSounded = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    const [count, feed, settings] = await Promise.all([
      api.unreadCount(),
      api.listNotifications(),
      api.getSettings()
    ]);

    setUnread(count);

    applyTheme(settings);

    const newest = feed[0]?.id ?? null;

    if (newest === null) return;

    // nothing sounds on the first load, only on something that arrived since
    if (lastSounded.current !== null && newest > lastSounded.current && settings.notificationSounds) {
      playNotificationSound(settings.soundVolume);
    }

    lastSounded.current = newest;
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useCoreEvent(EVENTS.feed, () => void refresh());
  // the settings screen lives in another webview, so a colour change arrives as an event
  useCoreEvent(EVENTS.settings, () => void refresh());
  // the feed can close without the bell being touched — the user clicks away, or a new server
  // webview takes it down — and the bell would otherwise stay drawn as active over nothing
  useCoreEvent<{ open: boolean }>(EVENTS.popup, ({ open }) => setOpen(open));

  const toggle = useCallback(async () => {
    try {
      const next = await api.togglePopup();

      setOpen(next);

      if (!next) return;

      await api.markNotificationsRead();
      await refresh();
    } catch {
      // the core owns this state, so leave the button as it was rather than guessing
    }
  }, [refresh]);

  return (
    <div className="bell-root">
      <button
        type="button"
        className={open ? 'icon-button active' : 'icon-button'}
        title="Notifications"
        aria-label={unread > 0 ? `Notifications, ${unread} unread` : 'Notifications'}
        onClick={toggle}
      >
        <BellIcon />
        {unread > 0 && !open ? <span className="badge">{unread > 99 ? '99+' : unread}</span> : null}
      </button>
    </div>
  );
};
