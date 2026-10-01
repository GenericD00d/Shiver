/**
 * The quick rail's page. tauri-plugin-shiver-rail shows it in a WebView of its own, over the server
 * page and out of that page's reach, and talks to it through one message channel (`shiverRail`,
 * which Android offers to this page's origin only): the plugin sends what to draw, the page answers
 * once it is drawn and says what the user chose.
 */

import { StrictMode, useCallback, useEffect, useLayoutEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';

import { Rail, type RailServer } from '../../shared/web/components/Rail';
import { applyTheme } from '../../shared/web/theme';
import type { Folder } from '../../shared/web/types';
import '../../shared/web/components/ui.css';
import './rail.css';

/** One server, as the core sends it (`tauri_plugin_shiver_rail::Tile`). */
type Tile = {
  id: string;
  name: string;
  folderId: string | null;
  position: number;
  unread: number;
  /** names the logo, when there is one */
  iconKey: string | null;
  /** the logo, sent only with a key this page has not been given yet */
  icon: string | null;
};

/** What to draw (`RailView`). */
type View = {
  themeColor: string;
  accentColor: string;
  textColor: string | null;
  /** the server whose page is behind the rail */
  current: string | null;
  servers: Tile[];
  folders: Folder[];
};

/** What the page says back. The plugin passes on only these kinds, and the core checks each. */
type Said =
  | { kind: 'ready' }
  | { kind: 'drawn'; seq: number }
  | { kind: 'open'; entryId: string }
  | { kind: 'dms' | 'add' | 'settings' | 'closed' }
  | { kind: 'folder'; folderId: string; expanded: boolean };

type Channel = {
  postMessage: (message: string) => void;
  onmessage: ((event: { data: string }) => void) | null;
};

declare global {
  interface Window {
    shiverRail?: Channel;
  }
}

/** How long the page waits for a frame before saying it has drawn anyway. */
const FRAME_WAIT_MS = 120;

const say = (message: Said) => window.shiverRail?.postMessage(JSON.stringify(message));

/** A logo the page can draw: a picture as a `data:` uri, nothing else. */
const isPicture = (icon: unknown): icon is string => typeof icon === 'string' && icon.startsWith('data:image/');

/** Each server's logo, kept across views: a key is sent with its picture only once. */
const logos = new Map<string, { key: string; icon: string }>();

const QuickRail = () => {
  const [drawing, setDrawing] = useState<{ view: View; seq: number } | null>(null);

  useEffect(() => {
    const channel = window.shiverRail;

    if (!channel) return;

    channel.onmessage = ({ data }) => {
      let message: { seq?: unknown; view?: View };

      try {
        message = JSON.parse(data);
      } catch {
        return;
      }

      const { seq, view } = message;

      if (typeof seq !== 'number' || !view || !Array.isArray(view.servers) || !Array.isArray(view.folders)) return;

      for (const tile of view.servers) {
        if (tile.iconKey && isPicture(tile.icon)) logos.set(tile.id, { key: tile.iconKey, icon: tile.icon });
        if (!tile.iconKey) logos.delete(tile.id);
      }

      setDrawing({ view, seq });
    };

    say({ kind: 'ready' });
  }, []);

  const view = drawing?.view;

  useLayoutEffect(() => {
    if (view) applyTheme(view);
  }, [view]);

  // Answered once the view is painted, so the plugin slides in a rail that is already drawn. A page
  // Android is not drawing gets no frames, so the answer does not wait on one for long.
  useEffect(() => {
    if (!drawing) return;

    let answered = false;
    const answer = () => {
      if (answered) return;

      answered = true;
      say({ kind: 'drawn', seq: drawing.seq });
    };
    const frame = requestAnimationFrame(() => requestAnimationFrame(answer));
    const fallback = window.setTimeout(answer, FRAME_WAIT_MS);

    return () => {
      cancelAnimationFrame(frame);
      window.clearTimeout(fallback);
    };
  }, [drawing]);

  const servers = useMemo<RailServer[]>(
    () =>
      (view?.servers ?? []).map((tile) => {
        const logo = logos.get(tile.id);

        return {
          id: tile.id,
          name: tile.name,
          folderId: tile.folderId,
          position: tile.position,
          icon: logo && logo.key === tile.iconKey ? logo.icon : null
        };
      }),
    [view]
  );

  const unread = useMemo(
    () => Object.fromEntries((view?.servers ?? []).map((tile) => [tile.id, tile.unread])),
    [view]
  );

  // opened and shut here at once; the core stores it and the next view agrees
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  useEffect(() => setExpanded({}), [view]);

  const folders = useMemo(
    () => (view?.folders ?? []).map((folder) => ({ ...folder, expanded: expanded[folder.id] ?? folder.expanded })),
    [view, expanded]
  );

  const open = useCallback(
    (id: string) => say(id === view?.current ? { kind: 'closed' } : { kind: 'open', entryId: id }),
    [view?.current]
  );

  if (!view) return null;

  return (
    <Rail
      servers={servers}
      folders={folders}
      activeId={view.current}
      screen={null}
      unread={unread}
      onOpen={open}
      onOpenDms={() => say({ kind: 'dms' })}
      onAdd={() => say({ kind: 'add' })}
      onSettings={() => say({ kind: 'settings' })}
      onToggleFolder={(folderId, next) => {
        setExpanded((current) => ({ ...current, [folderId]: next }));
        say({ kind: 'folder', folderId, expanded: next });
      }}
    />
  );
};

createRoot(document.getElementById('root') as HTMLElement).render(
  <StrictMode>
    <div className="touch">
      <QuickRail />
    </div>
  </StrictMode>
);
