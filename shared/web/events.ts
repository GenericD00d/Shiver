/**
 * The core's events, for Shiver's own pages in both apps. Each app names its events and binds the
 * hook to Tauri's `listen` in its own `events.ts`, since shared code resolves no packages but React.
 */

import { useEffect, useRef } from 'react';

type Listen = <T>(event: string, handler: (event: { payload: T }) => void) => Promise<() => void>;

/** `useCoreEvent` over `listen`. */
export function coreEventHook(listen: Listen) {
  /** Calls the latest `handler` with each payload of `event` for as long as the component is mounted. */
  return function useCoreEvent<T = unknown>(event: string, handler: (payload: T) => void) {
    const latest = useRef(handler);

    latest.current = handler;

    useEffect(() => {
      const pending = listen<T>(event, ({ payload }) => latest.current(payload));

      return () => void pending.then((unlisten) => unlisten()).catch(() => undefined);
    }, [event]);
  };
}
