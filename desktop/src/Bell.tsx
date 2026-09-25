import { useCallback, useEffect, useRef, useState } from 'react';

import { api } from './api';
import { EVENTS, useCoreEvent } from './events';
import { BellIcon } from './components/icons';
import { playNotificationSound } from './sounds';
import type { FeedSummary, Settings } from './types';
import { applyTheme } from '../../shared/web/theme';

/** The notification bell, in a fixed 48x48 webview; the feed opens in a separate webview below it. */
export const Bell = () => {
  const [unread, setUnread] = useState(0);
  const [open, setOpen] = useState(false);
  // the newest notification already sounded for; nothing sounds for what was there on launch
  const lastSounded = useRef<number | null>(null);
  const settings = useRef<Settings | null>(null);

  const loadSettings = useCallback(async () => {
    settings.current = await api.getSettings();
    applyTheme(settings.current);
  }, []);

  const take = useCallback(({ unread: count, newest }: FeedSummary) => {
    setUnread(count);

    if (newest === null) return;

    const sounds = settings.current?.notificationSounds;

    if (sounds && lastSounded.current !== null && newest > lastSounded.current) {
      playNotificationSound(settings.current?.soundVolume);
    }

    lastSounded.current = newest;
  }, []);

  useEffect(() => {
    void loadSettings();
    api.feedSummary().then(take).catch(() => undefined);
  }, [loadSettings, take]);

  useCoreEvent(EVENTS.feed, take);
  // the settings screen lives in another webview, so a colour change arrives as an event
  useCoreEvent(EVENTS.settings, () => void loadSettings());
  // the feed can close without the bell being touched — the user clicks away, or a new server
  // webview takes it down — and the bell would otherwise stay drawn as active over nothing
  useCoreEvent<{ open: boolean }>(EVENTS.popup, ({ open }) => setOpen(open));

  const toggle = useCallback(async () => {
    try {
      const next = await api.togglePopup();

      setOpen(next);

      if (!next) return;

      await api.markNotificationsRead();
    } catch {
      // the core owns this state, so leave the button as it was rather than guessing
    }
  }, []);

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
