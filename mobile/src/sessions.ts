import { useCallback, useEffect, useState } from 'react';

import { api } from './api';
import type { SessionStates } from './types';

const NONE: SessionStates = { signedOut: [], remembered: [], problems: {}, plugins: {} };

/** How often the settings screen reads them again, since a problem can appear without a count moving. */
const POLL_MS = 10_000;

/**
 * Each server's session state, read again whenever `trigger` changes (the unread counts, which move
 * when the core publishes) and every few seconds while `polling`; nothing while not `enabled` (the
 * screens over a server's page, which can call the core only while they are seen). `read` reads them now.
 */
export const useSessions = (trigger: unknown, polling: boolean, enabled = true) => {
  const [sessions, setSessions] = useState<SessionStates>(NONE);

  const read = useCallback(() => {
    api
      .sessionStates()
      .then(setSessions)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    if (enabled) read();
  }, [enabled, read, trigger]);

  useEffect(() => {
    if (!polling || !enabled) return;

    const timer = window.setInterval(read, POLL_MS);

    return () => window.clearInterval(timer);
  }, [enabled, polling, read]);

  return { sessions, read };
};
