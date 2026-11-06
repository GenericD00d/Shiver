import { useCallback, useEffect, useMemo, useState } from 'react';

import { api } from '../api';
import { EVENTS, useCoreEvent } from '../events';
import type { DmEntry } from '../types';
import { DmList } from '../../../shared/web/components/DmList';
import { dmKey, type DmRow } from '../../../shared/web/dms';

type Props = {
  /** opens the server the conversation is on, landing on that conversation */
  onOpen: (entryId: string, userName: string) => void;
};

/**
 * Every server's conversations, newest message first, in the list the desktop client draws too. Drawn here rather than in a server's page so no server learns who the user talks to
 * elsewhere.
 */
export const DirectMessages = ({ onOpen }: Props) => {
  const [dms, setDms] = useState<DmEntry[] | null>(null);

  const load = useCallback(() => void api.listDms().then(setDms, () => setDms([])), []);

  // reloaded whenever the core publishes, since servers connect after this screen mounts
  useEffect(load, [load]);
  useCoreEvent(EVENTS.unread, load);

  const rows = useMemo(
    () =>
      (dms ?? []).map(
        (dm): DmRow => ({
          key: dmKey(dm.entryId, dm.channelId),
          entryId: dm.entryId,
          channelId: dm.channelId,
          name: dm.userName,
          avatarUrl: dm.avatarUrl,
          serverName: dm.serverName,
          accountLabel: dm.accountLabel,
          lastMessageAt: dm.lastMessageAt
        })
      ),
    [dms]
  );

  if (dms === null) return <p className="hint">Looking…</p>;

  return <DmList rows={rows} touch onOpen={(row) => onOpen(row.entryId, row.name)} />;
};
