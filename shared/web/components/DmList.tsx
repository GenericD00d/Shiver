import { useMemo, useState } from 'react';

import { dmInitial, dmMeta, type DmRow, filterDms, sortDms } from '../dms';
import { relativeTime } from '../time';
import './dm-list.css';

type Props = {
  rows: readonly DmRow[];
  onOpen: (row: DmRow) => void;
  /** the conversation shown beside the list (desktop), marked as open */
  selectedKey?: string | null;
  /** finger-sized rows, and a search field Android does not zoom into */
  touch?: boolean;
};

/**
 * Every server's conversations in one list, shaped like Sharkord's own DM list, with the account and
 * server each belongs to. Both clients draw this one.
 */
export const DmList = ({ rows, onOpen, selectedKey = null, touch = false }: Props) => {
  const [query, setQuery] = useState('');
  const filtered = useMemo(() => sortDms(filterDms(rows, query)), [rows, query]);

  return (
    <div className={touch ? 'dm-list touch' : 'dm-list'}>
      <input
        className="dm-search"
        type="text"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Search"
        spellCheck={false}
        autoComplete="off"
      />

      {filtered.length === 0 ? (
        <p className="dm-empty">{rows.length === 0 ? 'No conversations yet.' : 'Nothing matches that.'}</p>
      ) : null}

      <div className="dm-items">
        {filtered.map((row) => (
          <DmItem key={row.key} row={row} selected={row.key === selectedKey} onOpen={onOpen} />
        ))}
      </div>
    </div>
  );
};

const DmItem = ({ row, selected, onOpen }: { row: DmRow; selected: boolean; onOpen: (row: DmRow) => void }) => {
  // a picture that will not load (its link expired) falls back to the initial, until a new link comes
  const [broken, setBroken] = useState<string | null>(null);
  const picture = row.avatarUrl && row.avatarUrl !== broken ? row.avatarUrl : null;

  return (
    <button type="button" className={selected ? 'dm-item selected' : 'dm-item'} onClick={() => onOpen(row)}>
      {picture ? (
        <img src={picture} alt="" draggable={false} onError={() => setBroken(picture)} />
      ) : (
        <span className="dm-avatar">{dmInitial(row.name)}</span>
      )}

      <span className="dm-item-body">
        <span className="dm-item-name">{row.name}</span>
        <span className="dm-item-meta">{dmMeta(row)}</span>
      </span>

      {row.lastMessageAt !== null ? <span className="dm-item-time">{relativeTime(row.lastMessageAt)}</span> : null}
    </button>
  );
};
