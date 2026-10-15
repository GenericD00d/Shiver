import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { api, errorMessage } from './api';
import type { ConfirmAction } from './components/ActionConfirm';
import { Boot, type BootState } from './components/Boot';
import { DirectMessages } from './components/DirectMessages';
import { ServerSignIn } from './components/ServerSignIn';
import { SettingsView } from './components/SettingsView';
import { chooseMenuItem, railMenu, runConfirmed } from './rail-menu';
import { useSessions } from './sessions';
import { AddServerForm } from '../../shared/web/components/AddServerForm';
import { Menu } from '../../shared/web/components/Menu';
import { Rail } from '../../shared/web/components/Rail';
import { RenameFolderForm } from '../../shared/web/components/RenameFolderForm';
import { UpdateNotice } from '../../shared/web/components/UpdateNotice';
import { applyTheme } from '../../shared/web/theme';
import {
  DEFAULT_ACCENT_COLOR,
  DEFAULT_THEME_COLOR,
  type Registry,
  type ServerEntry,
  type Settings
} from './types';
import type { MenuEntry } from '../../shared/web/menus';
import { byPosition, type RailRef, type RailStep, runRailSteps } from '../../shared/web/rail';

/**
 * Shiver's own screens. `boot` opens the last server used, or waits after `#home` (the quick rail
 * could not be drawn, so the swipe that asked for it lands here).
 */
type Screen = 'boot' | 'add' | 'settings' | 'signIn' | 'dms' | 'folder';

/** The screens the quick rail opens on Shiver's page, each by the fragment of the same name (it draws
 * the settings and the direct messages over the server's page itself). */
const RAIL_SCREENS = ['add'] as const;

type RailScreen = (typeof RAIL_SCREENS)[number];

const EMPTY: Registry = {
  servers: [],
  folders: [],
  muted: [],
  settings: {
    themeColor: DEFAULT_THEME_COLOR,
    accentColor: DEFAULT_ACCENT_COLOR,
    textColor: null,
    soundVolume: 100,
    minimiseAttachments: true,
    lastServerId: null,
    trustedLinks: [],
    pushServers: []
  }
};

/**
 * How long Shiver's page may be on its way to a server before its rail is drawn. The quick rail is
 * the one seen over a server, so a page only passing through (launch, a server chosen on the quick
 * rail) shows none; one that stays that long is slow, and the rail lets the user go elsewhere.
 */
const PASSING_THROUGH_MS = 2000;

/** The screens with a bar across the top (settings draws its own); the others are a card with its own title and Cancel. */
const TITLES: Partial<Record<Screen, string>> = {
  dms: 'Direct messages'
};

/**
 * What a fragment on Shiver's own URL may ask for: one of Shiver's screens (chosen on the quick
 * rail), to stay here (the quick rail could not be drawn), a server to open (chosen on the quick
 * rail, or its page reconnecting) or one that failed (the core). Any page can navigate here, so ids
 * are looked up in the registry and nothing else is taken from the URL.
 */
type Intent =
  | { kind: 'open' | 'failed'; id: string }
  | { kind: 'home' }
  | { kind: 'screen'; screen: RailScreen }
  | null;

const decode = (text: string) => {
  try {
    return decodeURIComponent(text);
  } catch {
    return '';
  }
};

const readIntent = (hash: string): Intent => {
  const value = hash.replace(/^#/, '');

  if (value.startsWith('open=')) return { kind: 'open', id: decode(value.slice('open='.length)) };
  if (value.startsWith('failed=')) return { kind: 'failed', id: decode(value.slice('failed='.length)) };

  const screen = RAIL_SCREENS.find((candidate) => candidate === value);

  if (screen) return { kind: 'screen', screen };

  return value === 'home' ? { kind: 'home' } : null;
};

/** The server Shiver reopens: the last one used, or the first. */
const lastUsed = (registry: Registry) => {
  const ordered = byPosition(registry.servers);

  return ordered.find((server) => server.id === registry.settings.lastServerId) ?? ordered[0];
};

export const App = () => {
  const [registry, setRegistry] = useState<Registry>(EMPTY);
  const [screen, setScreen] = useState<Screen>('boot');
  /** the server the sign-in screen is for */
  const [signingIn, setSigningIn] = useState<string | null>(null);
  /** the folder the rename screen is for */
  const [renamingFolder, setRenamingFolder] = useState<string | null>(null);
  /** the rail's menu, while it is open */
  const [menu, setMenu] = useState<{ entries: MenuEntry[]; at: { x: number; y: number } } | null>(null);
  const [boot, setBoot] = useState<BootState>({ kind: 'waiting' });
  // which `connect` is the latest, and what is on screen now, so an older one's answer does nothing
  const connecting = useRef(0);
  const screenRef = useRef(screen);
  const bootRef = useRef(boot);
  const [unread, setUnread] = useState<Record<string, number>>({});
  const [error, setError] = useState<string | null>(null);
  /** a newer release on offer, until turned down or put off */
  const [update, setUpdate] = useState<string | null>(null);

  useEffect(() => {
    screenRef.current = screen;
    bootRef.current = boot;
  }, [screen, boot]);

  useEffect(() => {
    api
      .updateAvailable()
      .then(setUpdate)
      .catch(() => undefined);

    // found after launch, by the core's own check
    const stop = api.onUpdate(setUpdate);

    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, []);

  const [icons, setIcons] = useState<Record<string, string>>({});
  const loadIcons = useCallback(() => void api.serverIcons().then(setIcons, () => undefined), []);

  useEffect(loadIcons, [loadIcons]);

  const servers = useMemo(
    () => byPosition(registry.servers).map((server) => ({ ...server, icon: icons[server.id] ?? null })),
    [registry.servers, icons]
  );

  const refresh = useCallback(async () => {
    const next = await api.listRegistry();

    setRegistry(next);

    return next;
  }, []);

  /** Runs a change, then re-reads the registry (which redraws everything). */
  const change = useCallback(
    async (run: () => Promise<unknown>) => {
      try {
        await run();
        await refresh();
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    [refresh]
  );

  /**
   * Hands the webview to a server once `/info` answers (a failed load would be a dead end: Shiver's
   * chrome is drawn by the server's page). `dmUser` asks the bridge to open that conversation.
   */
  const connect = useCallback(async (server: ServerEntry, dmUser?: string) => {
    const mine = ++connecting.current;
    // a later choice, or the user going elsewhere (another screen, or a question on this one),
    // while `/info` was answering
    const superseded = () =>
      connecting.current !== mine ||
      screenRef.current !== 'boot' ||
      bootRef.current.kind !== 'connecting' ||
      bootRef.current.server.id !== server.id;
    const starting: BootState = { kind: 'connecting', server };

    setScreen('boot');
    setBoot(starting);
    screenRef.current = 'boot';
    bootRef.current = starting;

    try {
      await api.probeServer(server.origin);
    } catch {
      if (!superseded()) setBoot({ kind: 'failed', server });

      return;
    }

    if (superseded()) return;

    try {
      await api.selectServer(server.id, dmUser);
    } catch (cause) {
      if (superseded()) return;

      // not left on a spinner: the failed screen offers to try again
      setError(errorMessage(cause));
      setBoot({ kind: 'failed', server });
    }
  }, []);

  /** Back to the last server used (Shiver's own pages are not somewhere to be left standing). */
  const resume = useCallback(
    async (registry?: Registry) => {
      const target = lastUsed(registry ?? (await refresh().catch(() => EMPTY)));

      if (target) {
        await connect(target);
      } else {
        setScreen('boot');
        setBoot({ kind: 'empty' });
      }
    },
    [connect, refresh]
  );

  /** Answers a `confirm` boot state. */
  const confirmAction = useCallback(
    async (confirmed: boolean) => {
      if (boot.kind !== 'confirm') return;

      if (confirmed) {
        const { action, server } = boot;

        try {
          await runConfirmed(action, server.id);
        } catch (cause) {
          setError(errorMessage(cause));
        }
      }

      await resume();
    },
    [boot, resume]
  );

  // Runs once. The fragment is cleared as it is read, so a reload does not repeat it.
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;

    started.current = true;

    const intent = readIntent(window.location.hash);

    if (intent) history.replaceState(null, '', window.location.pathname);

    void (async () => {
      let next: Registry;

      try {
        next = await refresh();
      } catch (cause) {
        setError(errorMessage(cause));
        setBoot({ kind: 'empty' });

        return;
      }

      if (intent?.kind === 'screen') {
        setScreen(intent.screen);

        return;
      }

      if (intent?.kind === 'home') {
        const left = lastUsed(next);

        if (left) {
          setBoot({ kind: 'home', server: left });
        } else {
          await resume(next);
        }

        return;
      }

      const namedId = intent?.kind === 'open' || intent?.kind === 'failed' ? intent.id : null;
      const named = next.servers.find((server) => server.id === namedId);

      if (named) {
        if (intent?.kind === 'failed') setBoot({ kind: 'failed', server: named });
        else await connect(named);

        return;
      }

      await resume(next);
    })();
  }, [connect, refresh, resume]);

  useEffect(() => {
    applyTheme(registry.settings);
  }, [registry.settings]);

  // Android's back button, from one of Shiver's own screens: back to the last server, opened by
  // Shiver (a step back through history would bring it without its session); from the boot
  // screen, the system's own
  useEffect(() => {
    window.__SHIVER_BACK__ = () => {
      if (screen === 'boot') return false;

      void resume();

      return true;
    };

    return () => {
      delete window.__SHIVER_BACK__;
    };
  }, [screen, resume]);

  // a server that must wait for another join counts down on the connecting screen
  useEffect(() => {
    const stop = api.onJoinWait(({ entryId, seconds }) =>
      setBoot((current) =>
        current.kind === 'connecting' && current.server.id === entryId
          ? { ...current, until: Date.now() + seconds * 1000 }
          : current
      )
    );

    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, []);

  // unread counts from the core's own connections: read once, then pushed
  useEffect(() => {
    api.unreadCounts().then(setUnread).catch(() => undefined);

    const stop = api.onUnread(setUnread);

    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, []);

  // session state per server, re-read whenever the counts change (which is when the core publishes)
  // and, while settings is open, every few seconds: problems can appear or clear without any count moving
  const { sessions, read: readSessions } = useSessions(unread, screen === 'settings');

  const signingInServer = servers.find((server) => server.id === signingIn) ?? null;

  const openById = useCallback(
    (id: string, dmUser?: string) => {
      const server = servers.find((candidate) => candidate.id === id);

      if (server) void connect(server, dmUser);
    },
    [connect, servers]
  );

  /** Straight into a server just added. */
  const handleAdded = useCallback(async (id: string) => {
    const next = await refresh();
    // by id: a server in a folder is numbered within it, so the last by position may not be new
    const added = next.servers.find((server) => server.id === id);

    if (added) {
      await connect(added);
    } else {
      setScreen('boot');
    }
  }, [connect, refresh]);

  /** Asks on the boot screen before a rail menu action runs. */
  const handleAsk = useCallback(
    (action: ConfirmAction, id: string) => {
      const server = servers.find((candidate) => candidate.id === id);

      if (!server) return;

      setScreen('boot');
      setBoot({ kind: 'confirm', action, server });
    },
    [servers]
  );

  const handleSettings = useCallback((settings: Settings) => void change(() => api.updateSettings(settings)), [change]);

  /** A server's or folder's menu, from the item model both clients share. */
  const openMenu = useCallback(
    (target: RailRef, at: { x: number; y: number }) => {
      const entries = railMenu(target, servers, registry.folders, sessions);

      if (entries) setMenu({ entries, at });
    },
    [registry.folders, servers, sessions]
  );

  const chooseFromMenu = useCallback(
    (id: string) =>
      chooseMenuItem(id, {
        open: openById,
        change,
        ask: handleAsk,
        signIn: (target) => {
          setSigningIn(target);
          setScreen('signIn');
        },
        rename: (target) => {
          setRenamingFolder(target);
          setScreen('folder');
        },
        iconsChanged: loadIcons
      }),
    [change, handleAsk, loadIcons, openById]
  );

  /** A drag's calls, in order, then the registry read again (which redraws the rail). */
  const applyDrop = useCallback(
    (steps: RailStep[]) => void change(() => runRailSteps(steps, api)),
    [change]
  );

  const renaming = registry.folders.find((folder) => folder.id === renamingFolder);

  const notices = (
    <>
      {error ? <p className="error app-error">{error}</p> : null}

      {/* Shiver cannot install packages itself (that would need `REQUEST_INSTALL_PACKAGES`), so "Get
          it" opens the releases page */}
      <UpdateNotice
        version={update}
        takeLabel="Get it"
        onTake={api.openReleases}
        onLater={() => setUpdate(null)}
        onSkip={api.skipUpdate}
      />
    </>
  );

  // on its way to a server (a wait for a join is a stay, not a pass)
  const passingThrough =
    screen === 'boot' && (boot.kind === 'waiting' || (boot.kind === 'connecting' && !boot.until));
  const [slow, setSlow] = useState(false);

  useEffect(() => {
    setSlow(false);

    if (!passingThrough) return;

    const timer = window.setTimeout(() => setSlow(true), PASSING_THROUGH_MS);

    return () => window.clearTimeout(timer);
  }, [passingThrough]);

  return (
    <div className="app touch">
      {passingThrough && !slow ? null : (
        <Rail
          servers={servers}
          folders={registry.folders}
          activeId={null}
          screen={screen === 'dms' || screen === 'add' || screen === 'settings' ? screen : null}
          unread={unread}
          onOpen={openById}
          onOpenDms={() => setScreen('dms')}
          onAdd={() => setScreen('add')}
          onSettings={() => setScreen('settings')}
          onToggleFolder={(id, expanded) => void change(() => api.setFolderExpanded(id, expanded))}
          onDrop={applyDrop}
          onMenu={openMenu}
        />
      )}

      <div className="main">
        {screen === 'settings' ? (
          <SettingsView
            registry={registry}
            servers={servers}
            sessions={sessions}
            onSessionsChanged={readSessions}
            onSave={handleSettings}
            change={change}
            onOpen={openById}
            onAsk={handleAsk}
            onAdd={() => setScreen('add')}
            onSignIn={(id) => {
              setSigningIn(id);
              setScreen('signIn');
            }}
            onBack={() => void resume(registry)}
            notices={notices}
          />
        ) : (
          <>
            {!TITLES[screen] ? null : (
              <header className="bar">
                <h1>{TITLES[screen]}</h1>

                <button type="button" className="ghost" onClick={() => void resume(registry)}>
                  Back
                </button>
              </header>
            )}

            <main className="content">
              {notices}

              {screen === 'boot' ? (
                <Boot
                  state={boot}
                  onRetry={connect}
                  onAdd={() => setScreen('add')}
                  onConfirm={(confirmed) => void confirmAction(confirmed)}
                />
              ) : null}

              {screen === 'folder' && renaming ? (
                <div className="modal-backdrop">
                  <RenameFolderForm
                    key={renaming.id}
                    name={renaming.name}
                    onSave={(name) => void change(() => api.renameFolder(renaming.id, name)).then(() => resume(registry))}
                    onCancel={() => void resume(registry)}
                  />
                </div>
              ) : null}

              {screen === 'add' ? (
                <div className="modal-backdrop">
                  <AddServerForm
                    hint={
                      <>
                        Shiver signs in for you, so the server opens straight into the app. Your password goes only to
                        this server, and both it and the session are kept in Android's encrypted store, under a key the
                        phone's Keystore holds — so Shiver can sign you in again when the session runs out, which
                        Sharkord makes it do every seven days. Take it back whenever you like by holding the server in
                        the rail.
                      </>
                    }
                    check={api.checkServer}
                    add={async ({ origin, identity, password, accountLabel }) => {
                      const entry = await api.addServer(origin, identity, password, accountLabel, !!password);

                      await handleAdded(entry.id);
                    }}
                    onCancel={servers.length > 0 ? () => void resume(registry) : null}
                  />
                </div>
              ) : null}

              {screen === 'dms' ? (
                <DirectMessages onOpen={openById} />
              ) : null}

              {screen === 'signIn' && signingInServer ? (
                <ServerSignIn
                  server={signingInServer}
                  remembered={sessions.remembered.includes(signingInServer.id)}
                  onDone={() => {
                    readSessions();
                    setScreen('settings');
                  }}
                  onCancel={() => setScreen('settings')}
                />
              ) : null}
            </main>
          </>
        )}
      </div>

      {menu ? <Menu entries={menu.entries} at={menu.at} onChoose={chooseFromMenu} onClose={() => setMenu(null)} /> : null}
    </div>
  );
};
