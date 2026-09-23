import { useCallback, useMemo, useState } from 'react';

import { api } from '../api';
import { MessagesIcon, PlusIcon, SettingsIcon } from './icons';
import { VoiceTile } from './VoiceTile';
import type { Folder, ServerEntry, ServerStatus, VoiceStatus } from '../types';

type Props = {
  servers: ServerEntry[];
  folders: Folder[];
  activeId: string | null;
  /** unread notifications per entry id; a server missing from it has none */
  unread: Record<string, number>;
  /** whether each server is up; a server missing from it has not been heard from yet */
  statuses: Record<string, ServerStatus>;
  settingsOpen: boolean;
  dmsOpen: boolean;
  /** the one call running across every server, or null when there is none */
  voice: VoiceStatus | null;
  onSelect: (id: string) => void;
  onAdd: () => void;
  onOpenSettings: () => void;
  onOpenDms: () => void;
  onRefresh: () => void;
};

/** One row of the rail: a top-level server, or a folder. Both share a single position space. */
type RailItem =
  | { kind: 'server'; id: string; position: number; server: ServerEntry }
  | { kind: 'folder'; id: string; position: number; folder: Folder; contents: ServerEntry[] };

/** Where a drop lands: before/after reorder; `into` makes a folder or moves into one. */
type DropZone = 'before' | 'after' | 'into';

/** What a drag is carrying, and what it is hovering. */
type DragState = {
  dragId: string;
  overId: string;
  zone: DropZone;
} | null;

/** The fixed band at a slot's centre that merges rather than reorders (same on 48px and 40px tiles). */
const MERGE_BAND_PX = 14;

const zoneFor = (event: React.DragEvent, allowInto: boolean): DropZone => {
  const rect = event.currentTarget.getBoundingClientRect();
  const offset = event.clientY - rect.top;
  const middle = rect.height / 2;

  if (allowInto && Math.abs(offset - middle) <= MERGE_BAND_PX / 2) return 'into';

  return offset < middle ? 'before' : 'after';
};

const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase() ?? '')
    .join('') || '?';

/** The four shown in a collapsed folder's tile. */
const FOLDER_PREVIEW_COUNT = 4;

/** Matches the bell, which is the other place Shiver counts unread. */
const badgeLabel = (count: number) => (count > 99 ? '99+' : String(count));

/** A red mark over a server that is not answering (cut out, so its icon shows through; per-server mask id). */
const OfflineOverlay = ({ id }: { id: string }) => {
  const maskId = `shiver-offline-${id}`;

  return (
    <span className="rail-offline" aria-hidden="true">
      <svg viewBox="0 0 48 48" width="48" height="48">
        <mask id={maskId}>
          <rect width="48" height="48" fill="#fff" />
          <rect x="21.5" y="11" width="5" height="17" rx="2.5" fill="#000" />
          <circle cx="24" cy="34.5" r="2.75" fill="#000" />
        </mask>

        {/* the icon is dimmed first, and only then reddened. without it the mark is cut out onto
            whatever the icon happens to be — and on a light or warm icon a red-on-orange
            exclamation is close to invisible, which is the one thing this must not be. */}
        <rect width="48" height="48" fill="#000" opacity="0.5" />
        <rect
          width="48"
          height="48"
          fill="#ef4444"
          opacity="0.7"
          mask={`url(#${maskId})`}
        />
      </svg>
    </span>
  );
};

export const ServerRail = ({
  servers,
  folders,
  activeId,
  unread,
  statuses,
  settingsOpen,
  dmsOpen,
  voice,
  onSelect,
  onAdd,
  onOpenSettings,
  onOpenDms,
  onRefresh
}: Props) => {
  const [drag, setDrag] = useState<DragState>(null);

  const items = useMemo<RailItem[]>(() => {
    const rows: RailItem[] = servers
      .filter((server) => !server.folderId)
      .map((server) => ({
        kind: 'server' as const,
        id: server.id,
        position: server.position,
        server
      }));

    for (const folder of folders) {
      rows.push({
        kind: 'folder' as const,
        id: folder.id,
        position: folder.position,
        folder,
        contents: servers
          .filter((server) => server.folderId === folder.id)
          .sort((a, b) => a.position - b.position)
      });
    }

    return rows.sort((a, b) => a.position - b.position);
  }, [folders, servers]);

  const handleContextMenu = useCallback((event: React.MouseEvent, serverId: string) => {
    event.preventDefault();

    api.showServerMenu(serverId).catch(() => undefined);
  }, []);

  const handleFolderContextMenu = useCallback((event: React.MouseEvent, folderId: string) => {
    event.preventDefault();

    api.showFolderMenu(folderId).catch(() => undefined);
  }, []);

  const handleDragStart = useCallback((event: React.DragEvent, id: string) => {
    event.dataTransfer.effectAllowed = 'move';
    // firefox refuses to start a drag without payload, and the id is what the drop needs anyway
    event.dataTransfer.setData('text/plain', id);

    setDrag({ dragId: id, overId: id, zone: 'into' });
  }, []);

  const handleDragOver = useCallback((event: React.DragEvent, overId: string, allowInto: boolean) => {
    // preventDefault unconditionally: without it the element refuses the drop, and gating it on
    // react state meant the first dragover events (which fire before the dragstart re-render has
    // landed) silently declined
    event.preventDefault();
    event.dataTransfer.dropEffect = 'move';

    const zone = zoneFor(event, allowInto);

    setDrag((current) =>
      current && current.dragId !== overId ? { ...current, overId, zone } : current
    );
  }, []);

  const handleDragEnd = useCallback(() => setDrag(null), []);

  /** Moves the dragged item next to the target within the top-level rail. */
  const reorderTopLevel = useCallback(
    async (sourceId: string, targetId: string, zone: DropZone) => {
      const ordered = items.map((item) => ({ kind: item.kind, id: item.id }));
      const from = ordered.findIndex((item) => item.id === sourceId);

      if (from < 0) return;

      const [moved] = ordered.splice(from, 1);
      const to = ordered.findIndex((item) => item.id === targetId);

      if (to < 0 || !moved) return;

      ordered.splice(zone === 'after' ? to + 1 : to, 0, moved);

      await api.reorderRail(ordered);
    },
    [items]
  );

  /** Moves the dragged server next to the target inside a folder. */
  const reorderInFolder = useCallback(
    async (sourceId: string, targetId: string, zone: DropZone, contents: ServerEntry[]) => {
      const ordered = contents.map((server) => server.id).filter((id) => id !== sourceId);
      const to = ordered.indexOf(targetId);

      if (to < 0) return;

      ordered.splice(zone === 'after' ? to + 1 : to, 0, sourceId);

      await api.reorderServers(ordered);
    },
    []
  );

  const handleDrop = useCallback(
    async (event: React.DragEvent, targetId: string, targetFolderId: string | null) => {
      event.preventDefault();
      event.stopPropagation();

      const sourceId = drag?.dragId ?? event.dataTransfer.getData('text/plain');
      const zone = drag?.zone ?? 'into';

      setDrag(null);

      if (!sourceId || sourceId === targetId) return;

      const source = servers.find((server) => server.id === sourceId);

      try {
        if (zone === 'into') {
          // onto a folder tile: move in. onto a server: make a folder of the two.
          const folder = folders.find((candidate) => candidate.id === targetId);

          if (folder) {
            await api.setServerFolder(sourceId, folder.id);
          } else {
            await api.createFolderWith('New folder', [targetId, sourceId]);
          }

          onRefresh();

          return;
        }

        // dragging between scopes: the server joins or leaves the folder it was dropped beside
        if (source && source.folderId !== targetFolderId) {
          await api.setServerFolder(sourceId, targetFolderId);
        }

        if (targetFolderId) {
          const contents = servers
            .filter((server) => server.folderId === targetFolderId || server.id === sourceId)
            .sort((a, b) => a.position - b.position);

          await reorderInFolder(sourceId, targetId, zone, contents);
        } else {
          await reorderTopLevel(sourceId, targetId, zone);
        }

        onRefresh();
      } catch {
        onRefresh();
      }
    },
    [drag, folders, onRefresh, reorderInFolder, reorderTopLevel, servers]
  );

  const handleToggleFolder = useCallback(
    async (folder: Folder) => {
      await api.setFolderExpanded(folder.id, !folder.expanded).catch(() => undefined);

      onRefresh();
    },
    [onRefresh]
  );

  const renderServer = (server: ServerEntry, inFolder: boolean) => {
    const classes = ['rail-item'];
    const hovering = drag?.overId === server.id ? drag.zone : null;
    const count = unread[server.id] ?? 0;
    const status = statuses[server.id] ?? 'connecting';

    if (server.id === activeId && !settingsOpen && !dmsOpen) classes.push('active');
    if (hovering === 'into') classes.push('merge-target');
    if (hovering === 'before') classes.push('insert-before');
    if (hovering === 'after') classes.push('insert-after');
    if (inFolder) classes.push('in-folder');

    return (
      // the drop target is the slot, not the tile: it reaches into the gaps between tiles, which
      // otherwise accept nothing and make a precise drop harder than it needs to be
      <div
        className="rail-slot"
        key={server.id}
        // a server already inside a folder cannot be merged into another: folders do not nest
        onDragOver={(event) => handleDragOver(event, server.id, !inFolder)}
        onDrop={(event) => handleDrop(event, server.id, server.folderId)}
      >
      <button
        type="button"
        className={classes.join(' ')}
        title={
          status === 'offline'
            ? `${server.name} — not responding`
            : server.accountLabel
              ? `${server.name} (${server.accountLabel})`
              : server.name
        }
        draggable
        onClick={() => onSelect(server.id)}
        onContextMenu={(event) => handleContextMenu(event, server.id)}
        onDragStart={(event) => handleDragStart(event, server.id)}
        onDragEnd={handleDragEnd}
      >
        {server.iconUrl ? (
          <img src={server.iconUrl} alt="" draggable={false} />
        ) : (
          initials(server.name)
        )}

        {count > 0 ? <span className="rail-badge">{badgeLabel(count)}</span> : null}

        {status === 'offline' ? <OfflineOverlay id={server.id} /> : null}

      </button>
      </div>
    );
  };

  const renderFolder = (item: Extract<RailItem, { kind: 'folder' }>) => {
    const { folder, contents } = item;
    const preview = contents.slice(0, FOLDER_PREVIEW_COUNT);
    // only while collapsed: an expanded folder shows each member's own badge just below, and
    // showing both would count the same messages twice on screen
    const count = folder.expanded
      ? 0
      : contents.reduce((total, server) => total + (unread[server.id] ?? 0), 0);
    const tileClasses = ['rail-folder-tile'];
    const hovering = drag?.overId === folder.id ? drag.zone : null;

    if (hovering === 'into') tileClasses.push('drop-target');
    if (hovering === 'before') tileClasses.push('insert-before');
    if (hovering === 'after') tileClasses.push('insert-after');
    if (folder.expanded) tileClasses.push('expanded');

    return (
      <div className="rail-folder" key={folder.id}>
        <div
          className="rail-slot"
          onDragOver={(event) => handleDragOver(event, folder.id, true)}
          onDrop={(event) => handleDrop(event, folder.id, null)}
        >
        <button
          type="button"
          className={tileClasses.join(' ')}
          title={folder.name}
          draggable
          onClick={() => handleToggleFolder(folder)}
          onContextMenu={(event) => handleFolderContextMenu(event, folder.id)}
          onDragStart={(event) => handleDragStart(event, folder.id)}
          onDragEnd={handleDragEnd}
        >
          {/* collapsed, the tile *is* the preview: up to four member icons in a 2x2 grid */}
          {folder.expanded ? (
            <span className="rail-folder-open" aria-hidden="true" />
          ) : (
            <span className="rail-folder-preview">
              {preview.map((server) => (
                <span className="rail-folder-mini" key={server.id}>
                  {server.iconUrl ? (
                    <img src={server.iconUrl} alt="" draggable={false} />
                  ) : (
                    initials(server.name).slice(0, 1)
                  )}
                </span>
              ))}
            </span>
          )}

          {count > 0 ? <span className="rail-badge">{badgeLabel(count)}</span> : null}
        </button>
        </div>

        {folder.expanded ? (
          <div className="rail-folder-contents">
            {contents.map((server) => renderServer(server, true))}
          </div>
        ) : null}
      </div>
    );
  };

  return (
    <nav className="rail" aria-label="Servers">
      <button
        type="button"
        className={`rail-item${dmsOpen ? ' active' : ''}`}
        title="Direct messages"
        onClick={onOpenDms}
      >
        <MessagesIcon />
      </button>

      <div className="rail-divider" />

      {items.map((item) =>
        item.kind === 'folder' ? renderFolder(item) : renderServer(item.server, false)
      )}

      <button type="button" className="rail-item rail-add" title="Add a server" onClick={onAdd}>
        <PlusIcon />
      </button>

      <div className="rail-spacer" />

      {/* sits above the settings button so the call, when there is one, is the last thing the eye
          reaches on the way down the rail */}
      {voice ? <VoiceTile status={voice} onOpenServer={onSelect} /> : null}

      <div className="rail-divider" />

      <button
        type="button"
        className={`rail-item${settingsOpen ? ' active' : ''}`}
        title="Shiver settings"
        onClick={onOpenSettings}
      >
        <SettingsIcon />
      </button>
    </nav>
  );
};
