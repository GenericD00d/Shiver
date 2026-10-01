import { useMemo } from 'react';

import { DmList } from '../../../shared/web/components/DmList';
import { dmKey, type DmRow } from '../../../shared/web/dms';
import type { DmEntry } from '../types';

type Props = {
  dms: DmEntry[];
  onOpen: (entryId: string, name: string, channelId: number) => void;
  onClose: () => void;
  /** the conversation currently rendered beside the list by its own server */
  openedKey: string | null;
  error: string | null;
};

/** The unified DM inbox (the list both clients draw), in a sidebar beside the conversation. */
export const DirectMessagesPanel = ({ dms, onOpen, onClose, openedKey, error }: Props) => {
  const rows = useMemo(
    () =>
      dms.map(
        ({ entryId, serverName, accountLabel, channel }): DmRow => ({
          key: dmKey(entryId, channel.channelId),
          entryId,
          channelId: channel.channelId,
          name: channel.name,
          avatarUrl: channel.iconUrl,
          serverName,
          accountLabel,
          lastMessageAt: channel.lastMessageAt
        })
      ),
    [dms]
  );

  return (
    <div className="dm-layout">
      <div className="dm-sidebar">
        <div className="dm-sidebar-header">
          <span>Direct messages</span>
          <button type="button" className="dm-close" title="Close" onClick={onClose}>
            ✕
          </button>
        </div>

        {error ? <p className="dm-error">{error}</p> : null}

        <DmList rows={rows} selectedKey={openedKey} onOpen={(row) => onOpen(row.entryId, row.name, row.channelId)} />
      </div>

      {/* empty while a conversation is open: the server's own webview covers this region */}
      {openedKey ? null : (
        <div className="dm-hint">
          <p>Pick a conversation to read it here.</p>
        </div>
      )}
    </div>
  );
};
