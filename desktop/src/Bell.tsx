import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useRef, useState } from 'react';

import { api } from './api';
import { BellIcon } from './components/icons';
import { playNotificationSound } from './sounds';
import { applyTheme } from '../../shared/web/theme';

const FEED_EVENT = 'shiver://feed';
const SETTINGS_EVENT = 'shiver://settings';
const POPUP_EVENT = 'shiver://popup';

/**
 * The notification bell.
 *
 * Its webview is a fixed 48x48 that never moves or resizes, so the bell cannot shift on screen no
 * matter what the feed does. The feed is a separate webview that opens beneath it.
 */
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
    refresh();

    const pending = [
      listen(FEED_EVENT, () => refresh()),
      // the settings screen lives in another webview, so a colour change arrives as an event
      listen(SETTINGS_EVENT, () => refresh())
    ];

    return () => {
      for (const handle of pending) {
        handle.then((unsubscribe) => unsubscribe()).catch(() => undefined);
      }
    };
  }, [refresh]);

  // the feed can close without the bell being touched — the user clicks away, or a new server
  // webview takes it down — and the bell would otherwise stay drawn as active over nothing
  useEffect(() => {
    const pending = listen<{ open: boolean }>(POPUP_EVENT, (event) => {
      setOpen(event.payload.open);
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, []);

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
