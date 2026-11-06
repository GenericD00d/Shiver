import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { api, errorMessage } from './api';
import { EVENTS, useCoreEvent } from './events';
import { useLive } from './live';
import { type AskPanel, useRailMenu } from './rail-menu';
import { DirectMessagesPanel } from './components/DirectMessagesPanel';
import { VoiceTile } from './components/VoiceTile';
import { SettingsPanel } from './components/SettingsPanel';
import { WelcomePanel } from './components/WelcomePanel';
import { AddServerForm } from '../../shared/web/components/AddServerForm';
import { Confirm } from '../../shared/web/components/Confirm';
import { Rail } from '../../shared/web/components/Rail';
import { RenameFolderForm } from '../../shared/web/components/RenameFolderForm';
import { Connecting as ConnectingView } from '../../shared/web/components/Connecting';
import { SignInForm } from '../../shared/web/components/SignInForm';
import { UpdateNotice } from '../../shared/web/components/UpdateNotice';
import { applyTheme } from '../../shared/web/theme';
import {
  DEFAULT_COLORS,
  DEFAULT_PAGES_KEPT,
  type NotificationTarget,
  type Registry,
  type ServerEntry,
  type Settings
} from './types';
import { dmKey } from '../../shared/web/dms';
import { byPosition, type RailStep, railOrder, runRailSteps } from '../../shared/web/rail';

/** A message clicked in the bell's feed. */
type OpenMessage = { entryId: string; channelId: number | null; isDm: boolean; author: string; target: NotificationTarget };

/**
 * Which Shiver surface owns the content area. Anything other than `server` means the active server's
 * webview is hidden and the shell is drawing the whole area.
 */
type Panel = 'server' | 'add' | 'settings' | 'dms' | 'folder' | 'connecting' | 'signin' | 'remove';

/** The server Shiver is waiting on, and whether it has given up on it. */
type Connecting = {
  entryId: string;
  serverName: string;
  failed: boolean;
};

/** How long a server may take before the spinner shows (avoids a flash for fast servers). */
const SPINNER_DELAY_MS = 400;

/** How long Shiver waits before it says the server did not answer. */
const CONNECT_TIMEOUT_MS = 20000;

const EMPTY_REGISTRY: Registry = {
  servers: [],
  folders: [],
  muted: [],
  settings: {
    ...DEFAULT_COLORS,
    colorPresets: [],
    notificationSounds: true,
    desktopNotifications: true,
    closeToTray: false,
    startAtLogin: false,
    soundVolume: 100,
    minimiseAttachments: true,
    spellcheck: true,
    lastServerId: null,
    trustedLinks: [],
    muteHotkey: null,
    pagesKept: DEFAULT_PAGES_KEPT
  }
};

export const App = () => {
  const [registry, setRegistry] = useState<Registry>(EMPTY_REGISTRY);
  /** the address a `sharkord://` link offered, filled into the add screen */
  const [linkedOrigin, setLinkedOrigin] = useState<string | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [panel, setPanel] = useState<Panel>('server');
  /** the server or folder a screen is about: one to remove, sign in to, or rename */
  const [subject, setSubject] = useState<string | null>(null);
  // `<entry id>:<channel id>` of the conversation shown beside the DM list
  const [openedDm, setOpenedDm] = useState<string | null>(null);
  const [dmError, setDmError] = useState<string | null>(null);
  // the conversation to show again when the inbox is reopened, rather than a blank pane
  const [lastDm, setLastDm] = useState<{ entryId: string; name: string } | null>(null);
  const [connecting, setConnecting] = useState<Connecting | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const { dms, unread, refreshFeed, voice, offline, notices, update, dismissUpdate } = useLive();

  /**
   * The server being waited on, in a ref so the once-registered ready listener sees the current
   * value; navigating elsewhere clears it, so a late ready event is ignored.
   */
  const pendingRef = useRef<string | null>(null);
  /** bumped by every server or panel the user opens, so an older one's late answer does nothing */
  const navigationRef = useRef(0);

  // openServer needs the current rail without taking it as a dependency: it is itself a dependency
  // of the boot effect, which must not re-run every time the registry is refreshed
  const serversRef = useRef<ServerEntry[]>([]);

  /** pending `SPINNER_DELAY_MS` timer, so a server that comes up in time shows no spinner */
  const spinnerTimer = useRef<number | null>(null);
  /** pending `CONNECT_TIMEOUT_MS` timer, after which Shiver says the server did not answer */
  const timeoutTimer = useRef<number | null>(null);

  const servers = useMemo(
    () => byPosition(registry.servers),
    [registry.servers]
  );

  const refresh = useCallback(async () => {
    const next = await api.listRegistry();

    setRegistry(next);

    return next;
  }, []);

  useEffect(() => {
    serversRef.current = registry.servers;
  }, [registry.servers]);

  const cancelWaiting = useCallback(() => {
    for (const timer of [spinnerTimer, timeoutTimer]) {
      if (timer.current === null) continue;

      window.clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  /** Hands the content area back to the server's own client. */
  const showServerNow = useCallback(async (id: string) => {
    await api.selectServer(id);

    cancelWaiting();
    pendingRef.current = null;

    setActiveId(id);
    setPanel('server');
    setConnecting(null);
    setError(null);
  }, [cancelWaiting]);

  /**
   * Opens a server: at once if its page is already connected (the kept-alive pages), otherwise
   * behind Shiver's connecting view until it is.
   */
  const openServer = useCallback(
    async (id: string) => {
      const mine = ++navigationRef.current;
      const superseded = () => navigationRef.current !== mine;

      try {
        const isReady = await api.prepareServer(id);

        if (superseded()) return;

        if (isReady) {
          await showServerNow(id);

          return;
        }

        // the shell keeps the area rather than handing it to a client that would only show its own
        // connecting state
        await api.showShell();

        if (superseded()) return;

        const serverName = serversRef.current.find((server) => server.id === id)?.name ?? 'the server';

        pendingRef.current = id;

        setActiveId(id);
        setError(null);

        // held back rather than shown at once: see SPINNER_DELAY_MS
        cancelWaiting();
        spinnerTimer.current = window.setTimeout(() => {
          spinnerTimer.current = null;

          // the user may have gone somewhere else, or the server may have arrived, while we waited
          if (pendingRef.current !== id) return;

          setConnecting({ entryId: id, serverName, failed: false });
          setPanel('connecting');
        }, SPINNER_DELAY_MS);

        timeoutTimer.current = window.setTimeout(() => {
          timeoutTimer.current = null;

          if (pendingRef.current !== id) return;

          setConnecting({ entryId: id, serverName, failed: true });
          setPanel('connecting');
        }, CONNECT_TIMEOUT_MS);
      } catch (cause) {
        if (superseded()) return;

        // the server webview is what would have covered the content area, so on failure the shell
        // has to take it back or the error would be reported into a region nobody can see
        await api.showShell().catch(() => undefined);

        cancelWaiting();
        pendingRef.current = null;

        setActiveId(null);
        setPanel('server');
        setConnecting(null);
        setError(errorMessage(cause));
      }
    },
    [cancelWaiting, showServerNow]
  );

  /** Covers the content area with one of Shiver's screens, about `about` if given. False on failure. */
  const openPanel = useCallback(async (next: Exclude<Panel, 'server'>, about?: string) => {
    navigationRef.current += 1;

    if (about !== undefined) setSubject(about);

    try {
      await api.showShell();

      // the user has gone somewhere else, so a server that connects now must not pull them back
      pendingRef.current = null;

      setPanel(next);
      setError(null);

      return true;
    } catch (cause) {
      setError(errorMessage(cause));

      return false;
    }
  }, []);

  /// reopens the inbox, restoring the conversation it was last showing
  const openDmPanel = useCallback(async () => {
    if (!(await openPanel('dms')) || !lastDm) return;

    await api.openDm(lastDm.entryId, lastDm.name).catch((cause) => setError(errorMessage(cause)));
  }, [lastDm, openPanel]);

  /// closing the inbox gives the conversation's page its channels back
  const closePanel = useCallback(async () => {
    if (panel === 'dms') {
      setOpenedDm(null);
      setLastDm(null);
      setDmError(null);

      await api.exitDmSplit().catch(() => undefined);
    }

    if (!activeId) {
      setPanel('server');

      return;
    }

    await openServer(activeId);
  }, [activeId, openServer, panel]);

  /**
   * A `sharkord://add-server` link (from a server's sign-in page): the server, if it is in the rail
   * already, else the add screen with its address filled in for the user to check. False when
   * there was no link.
   */
  const followServerLink = useCallback(
    async (known: Registry) => {
      const origin = await api.takeServerLink().catch(() => null);

      if (!origin) return false;

      const existing = known.servers.find((server) => server.origin === origin);

      if (existing) {
        await openServer(existing.id);
      } else {
        setLinkedOrigin(origin);
        await openPanel('add');
      }

      return true;
    },
    [openPanel, openServer]
  );

  useCoreEvent(EVENTS.serverLink, () => {
    refresh()
      .then(followServerLink)
      .catch((cause) => setError(errorMessage(cause)));
  });

  useEffect(() => {
    const boot = async () => {
      try {
        const next = await refresh();

        await refreshFeed();

        // started by a link: where it leads, rather than the last server
        if (await followServerLink(next)) return;

        const last = next.settings.lastServerId;
        const target = next.servers.find((server) => server.id === last) ?? next.servers[0];

        if (target) {
          await openServer(target.id);
        }
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setReady(true);
      }
    };

    boot();
  }, [followServerLink, openServer, refresh, refreshFeed]);

  // the swap out of Shiver's cached view and into the live client
  useCoreEvent<{ entryId: string }>(EVENTS.serverReady, ({ entryId }) => {
    if (pendingRef.current === entryId) showServerNow(entryId).catch(() => undefined);
  });

  /// opens the conversation beside Shiver's DM list, drawn by that server's own client
  const handleOpenDm = useCallback(
    async (entryId: string, name: string, channelId: number) => {
      navigationRef.current += 1;

      try {
        await api.openDm(entryId, name);

        setActiveId(entryId);
        setOpenedDm(dmKey(entryId, channelId));
        setLastDm({ entryId, name });
        setDmError(null);
        setError(null);
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    []
  );

  // Ctrl+1…9 and Ctrl+Alt+Up/Down, which the core registers only while Shiver is in front
  useCoreEvent<{ server?: number; step?: number }>(EVENTS.shortcut, ({ server, step }) => {
    const order = railOrder(registry.servers, registry.folders);
    const current = order.findIndex((entry) => entry.id === activeId);
    let target: ServerEntry | undefined;

    if (typeof server === 'number') {
      target = order[server - 1];
    } else if (typeof step === 'number' && order.length > 0) {
      const from = current === -1 ? (step > 0 ? -1 : 0) : current;

      target = order[(from + step + order.length) % order.length];
    }

    if (target) void openServer(target.id);
  });

  const handleAdded = useCallback(
    async (id: string) => {
      await refresh();
      await openServer(id);
    },
    [openServer, refresh]
  );

  const handleRemove = useCallback(
    async (id: string) => {
      try {
        await api.removeServer(id);

        const next = await refresh();

        await refreshFeed();

        if (pendingRef.current === id) pendingRef.current = null;

        if (activeId !== id) {
          await closePanel();

          return;
        }

        setActiveId(null);
        setConnecting(null);

        const fallback = next.servers[0];

        if (fallback) {
          await openServer(fallback.id);

          return;
        }

        await api.showShell();
        setPanel('server');
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    [activeId, closePanel, openServer, refresh, refreshFeed]
  );

  const handleRenameFolder = useCallback(
    async (id: string, name: string) => {
      try {
        await api.renameFolder(id, name);
        await refresh();

        setSubject(null);
        await closePanel();
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    [closePanel, refresh]
  );

  const handleSignIn = useCallback(
    async (id: string, identity: string, password: string, rememberPassword: boolean) => {
      await api.signInServer(id, identity, password, rememberPassword);

      setSubject(null);
      await refresh();

      // opened through the usual path, so the rebuilt page comes up behind the connecting screen
      await openServer(id);
    },
    [openServer, refresh]
  );

  const handleSettingsSaved = useCallback(
    async (settings: Settings) => {
      try {
        await api.updateSettings(settings);
        await refresh();

        return true;
      } catch (cause) {
        setError(errorMessage(cause));

        return false;
      }
    },
    [refresh]
  );

  // a conversation the page could not open. the page still shows whatever was there before, so
  // Shiver must stop marking the new one as open rather than quietly disagreeing with the screen.
  useCoreEvent<{ name: string }>(EVENTS.dmFailed, ({ name }) => {
    setOpenedDm(null);
    // reported in the DM sidebar, not the content area: the server's webview covers that
    setDmError(`Could not open the conversation with ${name}.`);
  });

  /** A server's page is asking for credentials: collect them here, so Shiver can renew the session. */
  useCoreEvent<{ entryId: string }>(EVENTS.signedOut, ({ entryId }) => {
    // only for the server being looked at or waited on: Shiver must not drag the user off
    // whatever they are doing because a background server lost its session
    if (activeId !== entryId && pendingRef.current !== entryId) return;

    void openPanel('signin', entryId);
  });

  // Clicking a message in the bell's feed. The feed is drawn in the popup's own webview, which
  // cannot switch servers, so it asks and this does it — through the same `openServer` everything
  // else goes through, so a server that is still coming up gets the same cover it always does.
  useCoreEvent<OpenMessage>(EVENTS.openMessage, async ({ entryId, channelId, isDm, author, target }) => {
    try {
      if (isDm) {
        // a conversation opens in the inbox beside Shiver's list, which is where DMs live
        await openPanel('dms');
        await api.openDm(entryId, author);
        setActiveId(entryId);
        setLastDm({ entryId, name: author });

        return;
      }

      await openServer(entryId);

      // after the server, not with it: the page has to exist before it can be told where to go,
      // and it retries on its own side for the case where it is still connecting
      // a plugin's notification has no channel, but its page can still be told of the click
      if (channelId !== null || target.pluginRef !== null) {
        await api.selectChannel(entryId, channelId, target);
      }
    } catch (cause) {
      setError(errorMessage(cause));
    }
  });

  const showMenu = useRailMenu({
    registry,
    refresh,
    refreshFeed,
    openServer,
    ask: (about: AskPanel, id: string) => openPanel(about, id)
  });

  /** A drag's calls, in order; the rail is redrawn from the registry either way. */
  const applyDrop = useCallback(
    (steps: RailStep[]) => void runRailSteps(steps, api).finally(() => void refresh().catch(() => undefined)),
    [refresh]
  );

  useEffect(() => {
    applyTheme(registry.settings);
  }, [registry.settings]);

  const railServers = useMemo(
    () => servers.map((server) => ({ ...server, icon: server.iconUrl })),
    [servers]
  );

  const renaming = registry.folders.find((folder) => folder.id === subject);
  const subjectServer = registry.servers.find((server) => server.id === subject);

  const showWelcome = ready && panel === 'server' && !activeId && !error;

  return (
    <div className="app">
      <Rail
        servers={railServers}
        folders={registry.folders}
        activeId={activeId}
        screen={panel === 'dms' || panel === 'settings' || panel === 'add' ? panel : null}
        unread={unread}
        offline={offline}
        notices={notices}
        onOpen={(id) => void openServer(id)}
        onOpenDms={() => void openDmPanel()}
        onAdd={() => void openPanel('add')}
        onSettings={() => void openPanel('settings')}
        onToggleFolder={(id, expanded) =>
          void api
            .setFolderExpanded(id, expanded)
            .catch(() => undefined)
            .then(refresh)
        }
        onDrop={applyDrop}
        onMenu={(target) => void showMenu(target).catch(() => undefined)}
      >
        {voice ? <VoiceTile status={voice} onOpenServer={openServer} /> : null}
      </Rail>

      <div className="main">
        <div className="content">
          {/* over the top of whatever is on screen, so it is seen on the launch it was found */}
          <UpdateNotice
            version={update}
            takeLabel="Install"
            busyLabel="Downloading…"
            // on success the installer takes over and this process ends, so only a failure comes back
            onTake={api.installUpdate}
            onLater={dismissUpdate}
            onSkip={api.skipUpdate}
          />

          {error ? (
            <div className="panel">
              <p className="error">{error}</p>
            </div>
          ) : null}

          {panel === 'add' ? (
            <div className="modal-backdrop">
              <AddServerForm
                key={linkedOrigin ?? ''}
                initialOrigin={linkedOrigin ?? ''}
                hint={
                  <>
                    Shiver signs in for you so the server opens straight into the app. Your password goes only to
                    this server, and Shiver keeps it in your operating system's credential store so it can sign you
                    in again when the session runs out — Sharkord's last a week and cannot be renewed. Take it back
                    whenever you like from the server's own menu.
                  </>
                }
                check={api.checkServer}
                add={async ({ origin, identity, password, accountLabel }) => {
                  const entry = await api.addServer(
                    origin,
                    identity ?? undefined,
                    password ?? undefined,
                    accountLabel ?? undefined,
                    !!password
                  );

                  setLinkedOrigin(null);
                  await handleAdded(entry.id);
                }}
                onCancel={
                  servers.length > 0
                    ? () => {
                        setLinkedOrigin(null);
                        void closePanel();
                      }
                    : null
                }
              />
            </div>
          ) : null}

          {panel === 'settings' ? (
            <SettingsPanel
              settings={registry.settings}
              onSave={handleSettingsSaved}
              onClose={closePanel}
            />
          ) : null}

          {panel === 'signin' && subjectServer ? (
            <div className="modal-backdrop">
              <SignInForm
                key={subjectServer.id}
                serverName={subjectServer.name}
                identity={subjectServer.identity}
                hint={
                  <>
                    Shiver keeps the session in your device&apos;s keychain and signs you in, so this server opens
                    straight into the app instead of its login page. Your password is kept only if you ask below.
                  </>
                }
                rememberHint={
                  <>
                    Stored in your operating system&apos;s credential store, so Shiver can sign in again by itself
                    when the session expires — which it does every seven days. Left unticked, Shiver keeps only the
                    session and asks you again when it runs out.
                  </>
                }
                // off unless the user says so (the add form keeps any password it is given)
                remembered={false}
                onSubmit={(identity, password, remember) => handleSignIn(subjectServer.id, identity, password, remember)}
                onCancel={closePanel}
              />
            </div>
          ) : null}

          {panel === 'folder' && renaming ? (
            <div className="modal-backdrop">
              <RenameFolderForm
                key={renaming.id}
                name={renaming.name}
                onSave={(name) => void handleRenameFolder(renaming.id, name)}
                onCancel={closePanel}
              />
            </div>
          ) : null}

          {/* asked first: the rail's menu removes a server, and everything Shiver keeps for it, in one click */}
          {panel === 'remove' && subjectServer ? (
            <div className="modal-backdrop">
              <Confirm
                question={`Remove ${subjectServer.name}?`}
                hint="Shiver forgets its sign-in, saved password, notifications and page storage. Nothing changes on the server."
                action="Remove"
                onConfirm={() => void handleRemove(subjectServer.id)}
                onCancel={closePanel}
              />
            </div>
          ) : null}

          {/* the page is covered meanwhile */}
          {panel === 'connecting' && connecting ? (
            <ConnectingView
              serverName={connecting.serverName}
              failed={connecting.failed}
              onRetry={() => void openServer(connecting.entryId)}
            />
          ) : null}

          {panel === 'dms' ? (
            <DirectMessagesPanel
              dms={dms}
              onOpen={handleOpenDm}
              onClose={closePanel}
              openedKey={openedDm}
              error={dmError}
            />
          ) : null}

          {showWelcome ? <WelcomePanel onAdd={() => openPanel('add')} /> : null}
        </div>
      </div>
    </div>
  );
};
