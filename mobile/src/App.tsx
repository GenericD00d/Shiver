import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { api, errorMessage } from './api';
import { AddServer } from './components/AddServer';
import { BackgroundNotifications } from './components/BackgroundNotifications';
import { Boot } from './components/Boot';
import { DirectMessages } from './components/DirectMessages';
import { Rail, type RailRef } from './components/Rail';
import { ServerList } from './components/ServerList';
import { Sessions } from './components/Sessions';
import { SignInServer } from './components/SignInServer';
import { SettingsScreen } from './components/SettingsScreen';
import { applyTheme } from './theme';
import {
  DEFAULT_ACCENT_COLOR,
  DEFAULT_THEME_COLOR,
  type Registry,
  type ServerEntry,
  type Settings
} from './types';

/**
 * Shiver's own screens.
 *
 * `boot` is where the app starts and where it returns: it opens the last server used rather than
 * showing a menu. The other two are destinations the rail navigates to and back out of, so there is
 * no home screen — a switcher whose front page is a list of one server was a step in the way.
 */
type Screen = 'boot' | 'add' | 'settings' | 'signIn' | 'dms';

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
 * What the fragment on Shiver's own url is allowed to ask for.
 *
 * The rail drawn inside a server's page cannot call Shiver — a page on a server's origin has no
 * Tauri IPC, which is the rule that keeps a server from reaching anything Shiver knows. So a tap on
 * that rail is a *navigation* back here carrying a fragment, and this is where it is read. It is
 * parsed rather than trusted: `open` is looked up in the registry Shiver already holds, so the worst
 * a bad fragment can name is a server the user already added.
 */
type Intent =
  | { kind: 'open'; id: string; dmUser?: string }
  | { kind: 'screen'; screen: Screen }
  /** an action from the rail's long-press menu inside a server's page, which cannot call Shiver */
  | { kind: 'do'; action: 'refresh' | 'remove'; id: string }
  | null;

const readIntent = (hash: string): Intent => {
  const value = hash.replace(/^#/, '');

  if (value.startsWith('open=')) {
    // `open=<id>` on its own, or with the conversation to land on: `open=<id>&dm=<name>`
    const [id, ...rest] = value.slice('open='.length).split('&');
    const dm = rest.find((part) => part.startsWith('dm='));

    return {
      kind: 'open',
      id: decodeURIComponent(id),
      dmUser: dm ? decodeURIComponent(dm.slice('dm='.length)) : undefined
    };
  }

  if (value === 'add') return { kind: 'screen', screen: 'add' };
  if (value === 'settings') return { kind: 'screen', screen: 'settings' };
  // the rail's panel, asking for the conversations it deliberately no longer holds itself
  if (value === 'dms') return { kind: 'screen', screen: 'dms' };

  if (value.startsWith('do=')) {
    const [action, id] = value.slice('do='.length).split(':');

    if ((action === 'refresh' || action === 'remove') && id) {
      return { kind: 'do', action, id: decodeURIComponent(id) };
    }
  }

  return null;
};

/** What the boot screen is currently doing. */
type BootState =
  | { kind: 'waiting' }
  | { kind: 'connecting'; server: ServerEntry }
  | { kind: 'failed'; server: ServerEntry }
  | { kind: 'empty' };

export const App = () => {
  const [registry, setRegistry] = useState<Registry>(EMPTY);
  const [screen, setScreen] = useState<Screen>('boot');
  /** the server the sign-in screen is for */
  const [signingIn, setSigningIn] = useState<string | null>(null);
  const [boot, setBoot] = useState<BootState>({ kind: 'waiting' });
  const [unread, setUnread] = useState<Record<string, number>>({});
  const [error, setError] = useState<string | null>(null);

  const servers = useMemo(
    () => [...registry.servers].sort((a, b) => a.position - b.position),
    [registry.servers]
  );

  const refresh = useCallback(async () => {
    const next = await api.listRegistry();

    setRegistry(next);

    return next;
  }, []);

  /** Stores the rail's new order after a drag, then re-reads it so every tile agrees. */
  const reorderRail = useCallback(
    async (ordered: RailRef[]) => {
      try {
        await api.reorderRail(ordered);
        await refresh();
      } catch (problem) {
        setError(String(problem));
      }
    },
    [refresh]
  );

  /** Runs a change to the rail's folders and re-reads the registry, which is what redraws it. */
  const folderAction = useCallback(
    async (run: () => Promise<unknown>) => {
      try {
        await run();
        await refresh();
      } catch (problem) {
        setError(String(problem));
      }
    },
    [refresh]
  );

  const open = useCallback(async (id: string, dms = false, dmUser?: string) => {
    try {
      // from here the webview belongs to the server until its rail brings the user back
      await api.openServer(id, dms, dmUser);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, []);

  /**
   * Opens a server, once it has answered.
   *
   * The reachability check is the difference between a spinner that resolves and a webview showing
   * the system's own "page could not be loaded" — which on mobile is a dead end, because Shiver's own
   * chrome is drawn *by* the page that failed to load. `/info` needs no session and is the same
   * request that added the server.
   */
  const connect = useCallback(
    async (server: ServerEntry, dmUser?: string) => {
      setScreen('boot');
      setBoot({ kind: 'connecting', server });

      try {
        await api.probeServer(server.origin);
      } catch {
        setBoot({ kind: 'failed', server });

        return;
      }

      // `dmUser` set means the user tapped a conversation in the rail's list rather than the server
      // itself, so the bridge is told to land them on it once that server's client is up
      await open(server.id, false, dmUser);
    },
    [open]
  );

  /** The server Shiver reopens: the last one used, or the first if there is no last. */
  const lastUsed = useCallback(
    (next: Registry) => {
      const ordered = [...next.servers].sort((a, b) => a.position - b.position);

      return ordered.find((server) => server.id === next.settings.lastServerId) ?? ordered[0];
    },
    []
  );

  // Runs once. The fragment is read *before* anything is opened and cleared as it is read, because
  // arriving here from the rail's settings tile must not be answered by reopening the server the
  // user just left — and a reload must not repeat whatever the fragment asked for.
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

      // a menu action, performed here because the rail that asked for it has no way to call Shiver
      if (intent?.kind === 'do') {
        try {
          if (intent.action === 'refresh') await api.refreshServerInfo(intent.id);
          if (intent.action === 'remove') await api.removeServer(intent.id);
        } catch (cause) {
          setError(errorMessage(cause));
        }

        const after = await refresh().catch(() => next);
        const target = lastUsed(after);

        // back to a server, since Shiver's own pages are not somewhere to be left standing
        if (target) {
          await connect(target);
        } else {
          setBoot({ kind: 'empty' });
        }

        return;
      }

      if (intent?.kind === 'open') {
        const named = next.servers.find((server) => server.id === intent.id);

        if (named) {
          await connect(named, intent.dmUser);

          return;
        }
      }

      const target = lastUsed(next);

      if (!target) {
        setBoot({ kind: 'empty' });

        return;
      }

      await connect(target);
    })();
  }, [connect, lastUsed, refresh]);

  useEffect(() => {
    applyTheme(registry.settings);
  }, [registry.settings]);

  // the core counts unread over its own connections to the servers with no page on screen, which
  // on mobile is every server but one. it is asked once and then pushes.
  useEffect(() => {
    api.listUnread().then(setUnread).catch(() => undefined);

    const stop = api.onUnread(setUnread);

    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, []);

  /**
   * Which servers are waiting to be signed in, and which Shiver can sign in by itself.
   *
   * Re-read whenever the counts change, because that is the moment the core publishes: a session it
   * could not renew shows up as one of these. Until this existed a refused session was silent —
   * the server simply stopped reporting, and the direct-message list quietly lost it.
   */
  const [signedOut, setSignedOut] = useState<string[]>([]);
  const [problems, setProblems] = useState<Record<string, string>>({});

  const [remembered, setRemembered] = useState<string[]>([]);

  const signingInServer = servers.find((server) => server.id === signingIn) ?? null;

  const readSessions = useCallback(() => {
    api.signedOutServers().then(setSignedOut).catch(() => undefined);
    api
      .watchProblems()
      .then((found) =>
        setProblems(Object.fromEntries(found.map((problem) => [problem.entryId, problem.reason])))
      )
      .catch(() => undefined);
    api.rememberedServers().then(setRemembered).catch(() => undefined);
  }, []);

  useEffect(readSessions, [readSessions, unread]);

  /**
   * Keeps the settings screen honest while it is open.
   *
   * Whether Shiver can watch a server is answered by the watch loop's next attempt, thirty seconds
   * away — not by anything the screen does. Without this the red line under a server stays after
   * the problem is gone, and appears late when a new one starts, because the effect above only runs
   * when an unread count moves and a server nobody is talking on never moves one.
   */
  useEffect(() => {
    if (screen !== 'settings') return;

    const timer = window.setInterval(readSessions, 10_000);

    return () => window.clearInterval(timer);
  }, [screen, readSessions]);

  /**
   * Lets one server send messages of any size, or takes that back.
   *
   * The problem line is not cleared here, deliberately: Shiver finds out whether this worked by
   * connecting, which is at most thirty seconds away, and the effect above re-reads the problems
   * when it does — so the row stops complaining because the server is being watched again, not
   * because a button was pressed.
   */
  const handleAcceptAnySize = useCallback(
    async (id: string, accept: boolean) => {
      try {
        await api.setServerAcceptsAnySize(id, accept);
        await refresh();
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    [refresh]
  );

  /**
   * Direct messages, from Shiver's own screens.
   *
   * It used to open the last server used and ask that server's client to show its own dm list,
   * there being nothing for Shiver to show. There is now: the cross-server list moved here when it
   * stopped being handed to server pages, so the tile shows it rather than delegating.
   */
  const handleDms = useCallback(() => setScreen('dms'), []);

  /** Leaving Shiver's own screens means going back to a server, there being nowhere else to go. */
  const handleBack = useCallback(() => {
    const target = lastUsed(registry);

    if (!target) {
      setScreen('boot');
      setBoot({ kind: 'empty' });

      return;
    }

    void connect(target);
  }, [connect, lastUsed, registry]);

  const handleAdded = useCallback(async () => {
    const next = await refresh();
    const added = [...next.servers].sort((a, b) => a.position - b.position).at(-1);

    // straight into what was just added, which is what adding a server was for
    if (added) {
      await connect(added);

      return;
    }

    setScreen('boot');
  }, [connect, refresh]);

  const handleRemove = useCallback(
    async (id: string) => {
      try {
        await api.removeServer(id);
        await refresh();
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    [refresh]
  );

  const handleSettings = useCallback(
    async (settings: Settings) => {
      try {
        await api.updateSettings(settings);
        await refresh();
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    [refresh]
  );

  return (
    <div className="app">
      <Rail
        servers={servers}
        activeId={null}
        screen={screen}
        unread={unread}
        onOpen={(id) => {
          const server = servers.find((candidate) => candidate.id === id);

          if (server) void connect(server);
        }}
        onOpenDms={handleDms}
        onAdd={() => setScreen('add')}
        onSettings={() => setScreen('settings')}
        onRefresh={(id) => {
          void api
            .refreshServerInfo(id)
            .then(() => refresh())
            .catch((cause) => setError(errorMessage(cause)));
        }}
        onRemove={(id) => void handleRemove(id)}
        onReorder={(ordered) => void reorderRail(ordered)}
        folders={registry.folders}
        onSetFolder={(id, folderId) => void folderAction(() => api.setServerFolder(id, folderId))}
        onCreateFolder={(memberIds) =>
          void folderAction(() => api.createFolderWith('New folder', memberIds))
        }
        onRenameFolder={(id, name) => void folderAction(() => api.renameFolder(id, name))}
        onDeleteFolder={(id) => void folderAction(() => api.deleteFolder(id))}
        onToggleFolder={(id, expanded) =>
          void folderAction(() => api.setFolderExpanded(id, expanded))
        }
      />

      <div className="main">
        {screen === 'boot' ? null : (
          <header className="bar">
            <h1>{TITLES[screen]}</h1>

            <button type="button" className="ghost" onClick={handleBack}>
              Back
            </button>
          </header>
        )}

        {error ? <p className="error">{error}</p> : null}

        <main className="content">
          {screen === 'boot' ? (
            <Boot state={boot} onRetry={connect} onAdd={() => setScreen('add')} />
          ) : null}

          {screen === 'add' ? <AddServer onAdded={handleAdded} /> : null}

          {screen === 'dms' ? (
            <DirectMessages
              onOpen={(entryId, userName) => {
                const server = servers.find((candidate) => candidate.id === entryId);

                // the same landing the rail's own rows used: open that server, and tell its client
                // which conversation to select once it is up
                if (server) void connect(server, userName);
              }}
            />
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
              <SettingsScreen settings={registry.settings} onSave={handleSettings} />

              {/* server management lives here now that there is no home screen to hold it */}
              <h2 className="section">Servers</h2>

              <ServerList
                servers={servers}
                onOpen={(id) => {
                  const server = servers.find((candidate) => candidate.id === id);

                  if (server) void connect(server);
                }}
                onRemove={handleRemove}
                onAdd={() => setScreen('add')}
                signedOut={signedOut}
                problems={problems}
                onAcceptAnySize={(id, accept) => void handleAcceptAnySize(id, accept)}
                remembered={remembered}
                onSignIn={(id: string) => {
                  setSigningIn(id);
                  setScreen('signIn');
                }}
              />

              {/* said plainly, because Shiver holding a session for every server is not something a
                  user would guess from the badges it produces */}
              <h2 className="section">Background sessions</h2>

              <Sessions onCleared={readSessions} />

              {/* the other half of being told about messages: not what Shiver holds, but who tells it
                  when it is not running at all */}
              <h2 className="section">Notifications while Shiver is closed</h2>

              <BackgroundNotifications />
            </>
          ) : null}
        </main>
      </div>
    </div>
  );
};
