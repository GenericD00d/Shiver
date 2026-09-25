import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef } from 'react';

/** What the core tells Shiver's own webviews. */
export const EVENTS = {
  feed: 'shiver://feed',
  settings: 'shiver://settings',
  popup: 'shiver://popup',
  menu: 'shiver://server-menu',
  openMessage: 'shiver://open-message',
  dmFailed: 'shiver://dm-failed',
  serverReady: 'shiver://server-ready',
  voice: 'shiver://voice',
  status: 'shiver://status',
  signedOut: 'shiver://signed-out',
  update: 'shiver://update'
} as const;

/** Calls the latest `handler` with each payload of `event` for as long as the component is mounted. */
export function useCoreEvent<T = unknown>(event: string, handler: (payload: T) => void) {
  const latest = useRef(handler);

  latest.current = handler;

  useEffect(() => {
    const pending = listen<T>(event, ({ payload }) => latest.current(payload));

    return () => void pending.then((unlisten) => unlisten()).catch(() => undefined);
  }, [event]);
}
