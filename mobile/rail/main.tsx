/**
 * The quick rail's page. tauri-plugin-shiver-rail shows it in a WebView of its own, over the server
 * page and out of that page's reach, and talks to it through one message channel (`channel.ts`):
 * the plugin sends what to draw, the page answers once it is drawn and says what the user chose.
 *
 * It is also Shiver's screens over the server page, so reaching them leaves that page loaded and
 * connected: a tile's menu and what it asks (a confirmation, a folder's new name, a sign-in), the
 * settings and the direct messages. For those it fills the overlay, and calls the core through the
 * same `api` Shiver's own page uses, carried over the channel.
 */

import { StrictMode, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';

import { onBack, onView, say } from './channel';
import { api, errorMessage } from '../src/api';
import { ActionConfirm, type ConfirmAction } from '../src/components/ActionConfirm';
import { DirectMessages } from '../src/components/DirectMessages';
import { ServerSignIn } from '../src/components/ServerSignIn';
import { SettingsView } from '../src/components/SettingsView';
import { chooseMenuItem, railMenu, runConfirmed } from '../src/rail-menu';
import { useSessions } from '../src/sessions';
import type { Registry, Settings } from '../src/types';
import { Menu } from '../../shared/web/components/Menu';
import { Rail, type RailServer } from '../../shared/web/components/Rail';
import { RenameFolderForm } from '../../shared/web/components/RenameFolderForm';
import type { MenuEntry } from '../../shared/web/menus';
import { byPosition, type RailRef, type RailStep, runRailSteps } from '../../shared/web/rail';
import { applyTheme } from '../../shared/web/theme';
import type { Folder } from '../../shared/web/types';
import '../../shared/web/components/ui.css';
import '../src/styles.css';
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

/** A screen the page fills the overlay with, the rail beside it. */
type Screen = 'settings' | 'dms';

/**
 * What is over the rail. A question asked from a screen goes back to it once answered (`from`); one
 * asked from the rail's menu, to the rail.
 */
type Layer =
  | { kind: 'menu'; entries: MenuEntry[]; at: { x: number; y: number } }
  | { kind: 'confirm'; action: ConfirmAction; entryId: string; from: Screen | null }
  | { kind: 'rename'; folderId: string }
  | { kind: 'signIn'; entryId: string; from: Screen | null }
  | { kind: 'error'; message: string; from: Screen | null }
  | { kind: Screen };

/** How long the page waits for a frame before saying it has drawn anyway. */
const FRAME_WAIT_MS = 120;
/** The rail's width; wider than this, the plugin has given the page the whole overlay. */
const RAIL_PX = 72;

/** A logo the page can draw: a picture as a `data:` uri, nothing else. */
const isPicture = (icon: unknown): icon is string => typeof icon === 'string' && icon.startsWith('data:image/');

/** Each server's logo, kept across views: a key is sent with its picture only once. */
const logos = new Map<string, { key: string; icon: string }>();

const isScreen = (layer: Layer | null): layer is { kind: Screen } =>
  layer?.kind === 'settings' || layer?.kind === 'dms';

/** The screen a layer was opened from, or is. */
const screenOf = (layer: Layer | null): Screen | null => {
  if (!layer) return null;
  if (isScreen(layer)) return layer.kind;

  return 'from' in layer ? layer.from : null;
};

/** What is left once a layer is done: the screen it was asked from, or the rail. */
const returnTo = (layer: Layer): Layer | null => {
  if (isScreen(layer)) return null;

  const from = screenOf(layer);

  return from ? { kind: from } : null;
};

const QuickRail = () => {
  const [drawing, setDrawing] = useState<{ view: View; seq: number } | null>(null);
  const [layer, setLayer] = useState<Layer | null>(null);
  const layerRef = useRef(layer);
  /** the WebView is the overlay's width rather than the rail's, so a menu can measure where it fits */
  const [wide, setWide] = useState(false);
  /** read when the rail's menu or a screen wants more than the view holds */
  const [registry, setRegistry] = useState<Registry | null>(null);

  layerRef.current = layer;

  useEffect(
    () =>
      onView((message) => {
        const { seq, reveal, view } = message as { seq: number; reveal?: unknown; view?: View };

        if (!view || !Array.isArray(view.servers) || !Array.isArray(view.folders)) return;

        for (const tile of view.servers) {
          if (tile.iconKey && isPicture(tile.icon)) logos.set(tile.id, { key: tile.iconKey, icon: tile.icon });
          if (!tile.iconKey) logos.delete(tile.id);
        }

        // sliding in again: the rail, not whatever was left open last time
        if (reveal === true) setLayer(null);

        setDrawing({ view, seq });
      }),
    []
  );

  useEffect(() => {
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

  // the whole overlay while anything is over the rail
  useEffect(() => {
    say({ kind: 'expand', full: layer !== null });
  }, [layer !== null]);

  useEffect(() => {
    const measure = () => setWide(window.innerWidth > RAIL_PX + 1);

    measure();
    window.addEventListener('resize', measure);

    return () => window.removeEventListener('resize', measure);
  }, []);

  const unread = useMemo(
    () => Object.fromEntries((view?.servers ?? []).map((tile) => [tile.id, tile.unread])),
    [view]
  );

  const { sessions, read: readSessions } = useSessions(unread, layer?.kind === 'settings', layer !== null);

  const refresh = useCallback(async () => {
    const next = await api.listRegistry();

    setRegistry(next);

    return next;
  }, []);

  // read again with each view while something is over the rail (a change redraws the rail)
  useEffect(() => {
    if (layer !== null) void refresh().catch(() => undefined);
  }, [layer !== null, view, refresh]);

  const fail = useCallback((cause: unknown) => {
    setLayer((current) => ({ kind: 'error', message: errorMessage(cause), from: screenOf(current) }));
  }, []);

  /** Runs a change, then reads everything again; the core redraws the rail. */
  const change = useCallback(
    async (run: () => Promise<unknown>) => {
      try {
        await run();
        await refresh();
      } catch (cause) {
        fail(cause);
      }
    },
    [fail, refresh]
  );

  const close = useCallback(() => {
    setLayer(null);
    say({ kind: 'closed' });
  }, []);

  useEffect(
    () =>
      onBack(() => {
        const current = layerRef.current;

        // a screen is left for the page behind, as its Back does
        if (!current || isScreen(current)) close();
        else setLayer(returnTo(current));
      }),
    [close]
  );

  const open = useCallback(
    (id: string, dmUser?: string) => {
      setLayer(null);
      say(id === view?.current && !dmUser ? { kind: 'closed' } : { kind: 'open', entryId: id, ...(dmUser ? { dmUser } : {}) });
    },
    [view?.current]
  );

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

  /** The registry's servers in order, with the logos the rail has (redrawn with each view). */
  const entries = useMemo(
    () => byPosition(registry?.servers ?? []).map((server) => ({ ...server, icon: logos.get(server.id)?.icon })),
    [registry, view]
  );

  // opened and shut here at once; the core stores it and the next view agrees
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  useEffect(() => setExpanded({}), [view]);

  const folders = useMemo(
    () => (view?.folders ?? []).map((folder) => ({ ...folder, expanded: expanded[folder.id] ?? folder.expanded })),
    [view, expanded]
  );

  /** A tile's menu, once the registry and sessions it is built from are read. */
  const openMenu = useCallback(
    (target: RailRef, at: { x: number; y: number }) => {
      void Promise.all([refresh(), api.sessionStates()]).then(([next, states]) => {
        const items = railMenu(target, next.servers, next.folders, states);

        if (items) setLayer({ kind: 'menu', entries: items, at });
      }, fail);
    },
    [fail, refresh]
  );

  const chooseFromMenu = useCallback(
    (id: string) => {
      setLayer(null);
      chooseMenuItem(id, {
        open: (entryId) => open(entryId),
        change,
        ask: (action, entryId) => setLayer({ kind: 'confirm', action, entryId, from: null }),
        signIn: (entryId) => setLayer({ kind: 'signIn', entryId, from: null }),
        rename: (folderId) => setLayer({ kind: 'rename', folderId }),
        // the core sends the new logo with the view it redraws
        iconsChanged: () => undefined
      });
    },
    [change, open]
  );

  const answer = useCallback(
    (confirmed: boolean) => {
      if (layer?.kind !== 'confirm') return;

      const { action, entryId } = layer;

      setLayer(returnTo(layer));

      if (confirmed) void change(() => runConfirmed(action, entryId));
    },
    [change, layer]
  );

  if (!view) return null;

  const screen = isScreen(layer) ? layer.kind : null;
  const target = layer && 'entryId' in layer ? entries.find((entry) => entry.id === layer.entryId) : undefined;

  return (
    <div className="app">
      <Rail
        servers={servers}
        folders={folders}
        activeId={screen ? null : view.current}
        screen={screen}
        unread={unread}
        onOpen={(id) => open(id)}
        onOpenDms={() => setLayer({ kind: 'dms' })}
        onAdd={() => say({ kind: 'add' })}
        onSettings={() => setLayer({ kind: 'settings' })}
        onToggleFolder={(folderId, next) => {
          setExpanded((current) => ({ ...current, [folderId]: next }));
          say({ kind: 'folder', folderId, expanded: next });
        }}
        onDrop={(steps: RailStep[]) => void change(() => runRailSteps(steps, api))}
        onMenu={openMenu}
      />

      {screen === 'settings' && !registry ? (
        <div className="main">
          <p className="hint content">Looking…</p>
        </div>
      ) : null}

      {screen === 'settings' && registry ? (
        <div className="main">
          <SettingsView
            registry={registry}
            servers={entries}
            sessions={sessions}
            onSessionsChanged={readSessions}
            onSave={(settings: Settings) => void change(() => api.updateSettings(settings))}
            change={change}
            onOpen={(id) => open(id)}
            onAsk={(action, entryId) => setLayer({ kind: 'confirm', action, entryId, from: 'settings' })}
            onAdd={() => say({ kind: 'add' })}
            onSignIn={(entryId) => setLayer({ kind: 'signIn', entryId, from: 'settings' })}
            onBack={close}
          />
        </div>
      ) : null}

      {screen === 'dms' ? (
        <div className="main">
          <header className="bar">
            <h1>Direct messages</h1>

            <button type="button" className="ghost" onClick={close}>
              Back
            </button>
          </header>

          <main className="content">
            <DirectMessages onOpen={(entryId, userName) => open(entryId, userName)} />
          </main>
        </div>
      ) : null}

      {layer?.kind === 'menu' && wide ? (
        <Menu entries={layer.entries} at={layer.at} onChoose={chooseFromMenu} onClose={() => setLayer(null)} />
      ) : null}

      {layer?.kind === 'confirm' && target ? (
        <ActionConfirm action={layer.action} serverName={target.name} onAnswer={answer} />
      ) : null}

      {layer?.kind === 'rename' && registry ? (
        <RenameLayer
          registry={registry}
          folderId={layer.folderId}
          onSave={(name) => {
            const { folderId } = layer;

            setLayer(null);
            void change(() => api.renameFolder(folderId, name));
          }}
          onCancel={() => setLayer(null)}
        />
      ) : null}

      {layer?.kind === 'signIn' && target ? (
        <ServerSignIn
          server={target}
          remembered={sessions.remembered.includes(layer.entryId)}
          onDone={() => {
            readSessions();
            setLayer(returnTo(layer));
          }}
          onCancel={() => setLayer(returnTo(layer))}
        />
      ) : null}

      {layer?.kind === 'error' ? (
        <div className="modal-backdrop">
          <div className="card" role="alertdialog" aria-label={layer.message}>
            <p className="error">{layer.message}</p>

            <div className="actions">
              <button type="button" className="primary" onClick={() => setLayer(returnTo(layer))}>
                OK
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
};

/** A folder's new name, over the rail. */
const RenameLayer = ({
  registry,
  folderId,
  onSave,
  onCancel
}: {
  registry: Registry;
  folderId: string;
  onSave: (name: string) => void;
  onCancel: () => void;
}) => {
  const folder = registry.folders.find((candidate) => candidate.id === folderId);

  if (!folder) return null;

  return (
    <div className="modal-backdrop">
      <RenameFolderForm key={folder.id} name={folder.name} onSave={onSave} onCancel={onCancel} />
    </div>
  );
};

createRoot(document.getElementById('root') as HTMLElement).render(
  <StrictMode>
    <div className="touch">
      <QuickRail />
    </div>
  </StrictMode>
);
