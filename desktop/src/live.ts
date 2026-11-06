/**
 * What the shell mirrors from the core, kept current by its events rather than by polling: the DM
 * inbox and each server's unread count, the call, each server's status and desktop api mismatch,
 * and a newer release on offer.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';

import { api } from './api';
import { EVENTS, useCoreEvent } from './events';
import type { DmEntry, Mismatch, ServerStatus, VoiceStatus } from './types';

/** What a server's tooltip says when its client speaks another version of Sharkord's desktop api. */
const MISMATCH_NOTICES: Record<Mismatch, string> = {
  'server-outdated': 'This server runs an older Sharkord: its desktop features are off until it is updated.',
  'app-outdated': 'This server needs a newer Shiver: its desktop features are off until you update.'
};

export function useLive() {
  const [dms, setDms] = useState<DmEntry[]>([]);
  const [unread, setUnread] = useState<Record<string, number>>({});
  const [voice, setVoice] = useState<VoiceStatus | null>(null);
  const [statuses, setStatuses] = useState<Record<string, ServerStatus>>({});
  const [mismatches, setMismatches] = useState<Record<string, Mismatch>>({});
  /** a newer release on offer; "Later" holds until a newer one is found (the bell keeps the offer) */
  const [update, setUpdate] = useState<string | null>(null);

  // the bell and its popup live in their own overlay webview. the shell tracks what the *rail*
  // shows: the DM inbox, and the unread badge on each server icon.
  const refreshFeed = useCallback(async () => {
    const [nextDms, counts] = await Promise.all([api.listDms(), api.unreadCounts()]);

    setDms(nextDms);
    setUnread(counts);
  }, []);

  // the core drains each live page on a timer and says so here, rather than the shell polling
  useCoreEvent(EVENTS.feed, () => void refreshFeed());

  // the core drives this rather than the shell polling: two of the three states are not events, a
  // server goes from connecting to offline by nothing happening for long enough
  useCoreEvent<Record<string, ServerStatus>>(EVENTS.status, setStatuses);
  useCoreEvent<Record<string, Mismatch>>(EVENTS.mismatch, setMismatches);

  const refreshVoice = useCallback(async () => {
    setVoice(await api.voiceStatus().catch(() => null));
  }, []);

  // a call can start, move or end on any server, including one nobody is looking at
  useEffect(() => {
    void refreshVoice();
  }, [refreshVoice]);

  useCoreEvent(EVENTS.voice, () => void refreshVoice());

  useEffect(() => {
    api
      .availableUpdate()
      .then(setUpdate)
      .catch(() => undefined);
  }, []);

  // found after launch, by the core's own check
  useCoreEvent<string>(EVENTS.update, setUpdate);

  const offline = useMemo(
    () => Object.keys(statuses).filter((id) => statuses[id] === 'offline'),
    [statuses]
  );
  const notices = useMemo(
    () =>
      Object.fromEntries(
        Object.entries(mismatches).map(([id, mismatch]) => [id, MISMATCH_NOTICES[mismatch]])
      ),
    [mismatches]
  );

  return {
    dms,
    unread,
    refreshFeed,
    voice,
    offline,
    notices,
    update,
    dismissUpdate: () => setUpdate(null)
  };
}
