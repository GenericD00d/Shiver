import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { api, errorMessage } from './api';
import { AddServer } from './components/AddServer';
import { BackgroundNotifications } from './components/BackgroundNotifications';
import { Boot, type BootState, type ConfirmAction } from './components/Boot';
import { DirectMessages } from './components/DirectMessages';
import { UpdateNotice } from './components/UpdateNotice';
import type { SettingsSection } from './components/SettingsScreen';
import { Rail, type RailRef } from './components/Rail';
import { ServerList } from './components/ServerList';
import { Sessions } from './components/Sessions';
import { SignInServer } from './components/SignInServer';
import { SettingsScreen } from './components/SettingsScreen';
import { applyTheme } from '../../shared/web/theme';
import {
  DEFAULT_ACCENT_COLOR,
  DEFAULT_THEME_COLOR,
  type Registry,
  type ServerEntry,
  type Settings
} from './types';
import { byPosition } from '../../shared/web/rail';

/** Shiver's own screens. `boot` opens the last server used; there is no home screen. */
type Screen = 'boot' | 'add' | 'settings' | 'signIn' | 'dms';

/** The settings sections, in the order they are listed. */
const SETTINGS_SECTIONS: { id: SettingsSection | 'servers'; label: string }[] = [
  { id: 'appearance', label: 'Appearance' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'servers', label: 'Servers' },
  { id: 'about', label: 'About' }
];

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
    pushServers: []
  }
};

const TITLES: Record<Exclude<Screen, 'boot'>, string> = {
  add: 'Add a server',
  settings: 'Settings',
  signIn: 'Sign in',
  dms: 'Direct messages'
};

/**
 * What a fragment on Shiver's own URL may ask for. The rail inside a server's page has no IPC, so
 * its taps navigate here; any page can do that, so ids are looked up in the registry and
 * destructive actions are confirmed by the user on this page.
 */
type Intent =
  | { kind: 'open' | 'failed'; id: string }
  | { kind: 'screen'; screen: Screen }
  | { kind: 'do'; action: 'refresh' | ConfirmAction; id: string }
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
  if (value === 'add') return { kind: 'screen', screen: 'add' };
  if (value === 'settings') return { kind: 'screen', screen: 'settings' };
  if (value === 'dms') return { kind: 'screen', screen: 'dms' };

  if (value.startsWith('do=')) {
    const [action, id] = value.slice('do='.length).split(':');

    if ((action === 'refresh' || action === 'remove' || action === 'forgetpw' || action === 'logout') && id) {
      return { kind: 'do', action, id: decode(id) };
    }
  }

  return null;
};

/** The server Shiver reopens: the last one used, or the first. */
const lastUsed = (registry: Registry) => {
  const ordered = byPosition(registry.servers);

  return ordered.find((server) => server.id === registry.settings.lastServerId) ?? ordered[0];
};

export const App = () => {
  const [registry, setRegistry] = useState<Registry>(EMPTY);
  const [screen, setScreen] = useState<Screen>('boot');
  const [settingsSection, setSettingsSection] = useState<SettingsSection | 'servers'>('appearance');
  const [settingsDrawerOpen, setSettingsDrawerOpen] = useState(false);
  /** the server the sign-in screen is for */
  const [signingIn, setSigningIn] = useState<string | null>(null);
  const [boot, setBoot] = useState<BootState>({ kind: 'waiting' });
  const [unread, setUnread] = useState<Record<string, number>>({});
  const [error, setError] = useState<string | null>(null);

  const servers = useMemo(
    () => byPosition(registry.servers),
    [registry.servers]
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
    setScreen('boot');
    setBoot({ kind: 'connecting', server });

    try {
      await api.probeServer(server.origin);
    } catch {
      setBoot({ kind: 'failed', server });

      return;
    }

    try {
      await api.selectServer(server.id, dmUser);
    } catch (cause) {
      setError(errorMessage(cause));
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
          const run = { remove: api.removeServer, forgetpw: api.forgetPassword, logout: api.logOutServer }[action];

          await run(server.id);
        } catch (cause) {
          setError(errorMessage(cause));
        }
      }

      await resume();
    },
    [boot, resume]
  );

  // Runs once. The fragment is cleared as it is read, so a reload does not repeat it and arriving
  // from the rail's settings tile does not reopen the server just left.
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

      if (intent?.kind === 'do') {
        const server = next.servers.find((candidate) => candidate.id === intent.id);

        if (server && intent.action !== 'refresh') {
          setBoot({ kind: 'confirm', action: intent.action, server });

          return;
        }

        if (server) {
          await api.refreshServerInfo(server.id).catch((cause) => setError(errorMessage(cause)));
        }

        await resume();

        return;
      }

      const named = next.servers.find((server) => server.id === intent?.id);

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

  // unread counts from the core's own connections: read once, then pushed
  useEffect(() => {
    api.unreadCounts().then(setUnread).catch(() => undefined);

    const stop = api.onUnread(setUnread);

    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, []);

  // session state per server, re-read whenever the counts change (which is when the core publishes)
  const [signedOut, setSignedOut] = useState<string[]>([]);
  const [problems, setProblems] = useState<Record<string, string>>({});
  const [remembered, setRemembered] = useState<string[]>([]);
  /** companion plugin version per connected server */
  const [plugins, setPlugins] = useState<Record<string, string | null>>({});

  const signingInServer = servers.find((server) => server.id === signingIn) ?? null;

  const readSessions = useCallback(() => {
    api
      .sessionStates()
      .then((states) => {
        setSignedOut(states.signedOut);
        setRemembered(states.remembered);
        setProblems(states.problems);
        setPlugins(states.plugins);
      })
      .catch(() => undefined);
  }, []);

  useEffect(readSessions, [readSessions, unread]);

  // while settings is open, problems can appear or clear without any count moving
  useEffect(() => {
    if (screen !== 'settings') return;

    const timer = window.setInterval(readSessions, 10_000);

    return () => window.clearInterval(timer);
  }, [screen, readSessions]);

  const openById = useCallback(
    (id: string, dmUser?: string) => {
      const server = servers.find((candidate) => candidate.id === id);

      if (server) void connect(server, dmUser);
    },
    [connect, servers]
  );

  /** Straight into a server just added. */
  const handleAdded = useCallback(async () => {
    const next = await refresh();
    const added = byPosition(next.servers).at(-1);

    if (added) {
      await connect(added);
    } else {
      setScreen('boot');
    }
  }, [connect, refresh]);

  const handleRemove = useCallback((id: string) => void change(() => api.removeServer(id)), [change]);
  const handleSettings = useCallback((settings: Settings) => void change(() => api.updateSettings(settings)), [change]);

  return (
    <div className="app">
      <Rail
        servers={servers}
        activeId={null}
        screen={screen}
        unread={unread}
        onOpen={openById}
        onOpenDms={() => setScreen('dms')}
        onAdd={() => setScreen('add')}
        onSettings={() => setScreen('settings')}
        onRefresh={(id) => void change(() => api.refreshServerInfo(id))}
        onRemove={handleRemove}
        onReorder={(ordered: RailRef[]) => void change(() => api.reorderRail(ordered))}
        folders={registry.folders}
        onSetFolder={(id, folderId) => void change(() => api.setServerFolder(id, folderId))}
        onCreateFolder={(memberIds) => void change(() => api.createFolderWith('New folder', memberIds))}
        onRenameFolder={(id, name) => void change(() => api.renameFolder(id, name))}
        onDeleteFolder={(id) => void change(() => api.deleteFolder(id))}
        onToggleFolder={(id, expanded) => void change(() => api.setFolderExpanded(id, expanded))}
      />

      <div className="main">
        {screen === 'boot' ? null : (
          <header className="bar">
            {screen === 'settings' ? (
              <button
                type="button"
                className="ghost settings-menu"
                aria-label="Settings sections"
                aria-expanded={settingsDrawerOpen}
                onClick={() => setSettingsDrawerOpen((open) => !open)}
              >
                ☰
              </button>
            ) : null}

            <h1>{TITLES[screen]}</h1>

            <button type="button" className="ghost" onClick={() => void resume(registry)}>
              Back
            </button>
          </header>
        )}

        {error ? <p className="error">{error}</p> : null}

        <main className="content">
          <UpdateNotice />

          {screen === 'boot' ? (
            <Boot
              state={boot}
              onRetry={connect}
              onAdd={() => setScreen('add')}
              onConfirm={(confirmed) => void confirmAction(confirmed)}
            />
          ) : null}

          {screen === 'add' ? <AddServer onAdded={handleAdded} /> : null}

          {screen === 'dms' ? (
            <DirectMessages onOpen={openById} />
          ) : null}

          {screen === 'signIn' && signingInServer ? (
            <SignInServer
              server={signingInServer}
              remembered={remembered.includes(signingInServer.id)}
              onDone={() => {
                readSessions();
                setScreen('settings');
              }}
              onCancel={() => setScreen('settings')}
            />
          ) : null}

          {screen === 'settings' ? (
            <>
              {/* sections in a drawer over a scrim, as Sharkord's own settings do on a phone */}
              {settingsDrawerOpen ? (
                <div
                  className="settings-scrim"
                  onClick={() => setSettingsDrawerOpen(false)}
                  aria-hidden="true"
                />
              ) : null}

              <nav
                className={settingsDrawerOpen ? 'settings-drawer open' : 'settings-drawer'}
                aria-label="Settings sections"
              >
                {SETTINGS_SECTIONS.map((entry) => (
                  <button
                    key={entry.id}
                    type="button"
                    className={
                      entry.id === settingsSection ? 'settings-section active' : 'settings-section'
                    }
                    aria-current={entry.id === settingsSection}
                    onClick={() => {
                      setSettingsSection(entry.id);
                      setSettingsDrawerOpen(false);
                    }}
                  >
                    {entry.label}
                  </button>
                ))}
              </nav>

              <h2 className="section">
                {SETTINGS_SECTIONS.find((entry) => entry.id === settingsSection)?.label}
              </h2>

              {settingsSection === 'appearance' || settingsSection === 'about' ? (
                <SettingsScreen
                  settings={registry.settings}
                  onSave={handleSettings}
                  section={settingsSection}
                />
              ) : null}

              {settingsSection === 'notifications' ? (
                <>
                  <SettingsScreen
                    settings={registry.settings}
                    onSave={handleSettings}
                    section="notifications"
                  />

                  <h2 className="section">While Shiver is closed</h2>

                  <BackgroundNotifications />
                </>
              ) : null}
            </>
          ) : null}

          {screen === 'settings' && settingsSection === 'servers' ? (
            <>
              <ServerList
                servers={servers}
                onOpen={openById}
                onRemove={handleRemove}
                onAdd={() => setScreen('add')}
                signedOut={signedOut}
                problems={problems}
                onAcceptAnySize={(id, accept) => void change(() => api.setAcceptAnySize(id, accept))}
                remembered={remembered}
                plugins={plugins}
                onSignIn={(id: string) => {
                  setSigningIn(id);
                  setScreen('signIn');
                }}
              />

              <h2 className="section">Background sessions</h2>

              <Sessions onCleared={readSessions} />
            </>
          ) : null}
        </main>
      </div>
    </div>
  );
};
