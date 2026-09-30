import { useEffect, useMemo, useState } from 'react';

import { api } from '../api';
import type { DmEntry } from '../types';
import { DmList } from '../../../shared/web/components/DmList';
import { dmKey, type DmRow } from '../../../shared/web/dms';

type Props = {
  /** opens the server the conversation is on, landing on that conversation */
  onOpen: (entryId: string, userName: string) => void;
};

/**
 * Every server's conversations, newest first (`collect_dms`), in the list the desktop client draws
 * too. Drawn here rather than in a server's page so no server learns who the user talks to
 * elsewhere.
 */
export const DirectMessages = ({ onOpen }: Props) => {
  const [dms, setDms] = useState<DmEntry[] | null>(null);

  /** Reloaded whenever the core publishes, since servers connect after this screen mounts. */
  useEffect(() => {
    let live = true;

    const load = () => {
      api
        .listDms()
        .then((next) => {
          if (live) setDms(next);
        })
        .catch(() => {
          if (live) setDms([]);
        });
    };

    load();

    const pending = api.onUnread(load);

    return () => {
      live = false;
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, []);

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
