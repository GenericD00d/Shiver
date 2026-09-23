import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { api, errorMessage } from './api';
import { AddServerPanel } from './components/AddServerPanel';
import { UpdateNotice } from './components/UpdateNotice';
import { ConnectingPanel } from './components/ConnectingPanel';
import { DirectMessagesPanel } from './components/DirectMessagesPanel';
import { RenameFolderPanel } from './components/RenameFolderPanel';
import { ServerRail } from './components/ServerRail';
import { SettingsPanel } from './components/SettingsPanel';
import { SignInPanel } from './components/SignInPanel';
import { WelcomePanel } from './components/WelcomePanel';
import { applyTheme } from '../../shared/web/theme';
import {
  DEFAULT_ACCENT_COLOR,
  DEFAULT_PAGES_KEPT,
  DEFAULT_THEME_COLOR,
  type DmEntry,
  type Registry,
  type ServerEntry,
  type ServerStatus,
  type Settings,
  type VoiceStatus
} from './types';
import { byPosition } from '../../shared/web/rail';

/**
 * Which Shiver surface owns the content area. Anything other than `server` means the active server's
 * webview is hidden and the shell is drawing the whole area.
 */
type Panel = 'server' | 'add' | 'settings' | 'dms' | 'folder' | 'connecting' | 'signin';

/** The server Shiver is waiting on, and whether it has given up on it. */
type Connecting = {
  entryId: string;
  serverName: string;
  failed: boolean;
};

const FEED_EVENT = 'shiver://feed';
const MENU_EVENT = 'shiver://server-menu';
const OPEN_MESSAGE_EVENT = 'shiver://open-message';
const DM_FAILED_EVENT = 'shiver://dm-failed';
const SERVER_READY_EVENT = 'shiver://server-ready';
const VOICE_EVENT = 'shiver://voice';
const STATUS_EVENT = 'shiver://status';
const SIGNED_OUT_EVENT = 'shiver://signed-out';

/** How long a server may take before the spinner shows (avoids a flash for fast servers). */
const SPINNER_DELAY_MS = 400;

/** How long Shiver waits before it says the server did not answer. */
const CONNECT_TIMEOUT_MS = 20000;

const EMPTY_REGISTRY: Registry = {
  servers: [],
  folders: [],
  muted: [],
  settings: {
    themeColor: DEFAULT_THEME_COLOR,
    accentColor: DEFAULT_ACCENT_COLOR,
    textColor: null,
    notificationSounds: true,
    soundVolume: 100,
    minimiseAttachments: true,
    lastServerId: null,
    muteHotkey: null,
    pagesKept: DEFAULT_PAGES_KEPT
  }
};

export const App = () => {
  const [registry, setRegistry] = useState<Registry>(EMPTY_REGISTRY);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [panel, setPanel] = useState<Panel>('server');
  const [dms, setDms] = useState<DmEntry[]>([]);
  const [unread, setUnread] = useState<Record<string, number>>({});
  // `<entry id>:<channel id>` of the conversation the split is currently showing
  const [openedDm, setOpenedDm] = useState<string | null>(null);
  const [dmError, setDmError] = useState<string | null>(null);
  // the conversation to restore when the inbox is reopened. its webview is only hidden when the
  // user goes off to a server, so coming back should land on it rather than on a blank pane.
  const [lastDm, setLastDm] = useState<{ entryId: string; name: string } | null>(null);
  const [renamingFolder, setRenamingFolder] = useState<string | null>(null);
  const [signingIn, setSigningIn] = useState<string | null>(null);
  const [voice, setVoice] = useState<VoiceStatus | null>(null);
  const [statuses, setStatuses] = useState<Record<string, ServerStatus>>({});
  const [connecting, setConnecting] = useState<Connecting | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);

  /**
   * The server being waited on, in a ref so the once-registered ready listener sees the current
   * value; navigating elsewhere clears it, so a late ready event is ignored.
   */
  const pendingRef = useRef<string | null>(null);

  // openServer needs the current rail without taking it as a dependency: it is itself a dependency
  // of the boot effect, which must not re-run every time the registry is refreshed
  const serversRef = useRef<ServerEntry[]>([]);

  /** the shown server, for listeners registered once that must not read a stale value */
  const activeIdRef = useRef<string | null>(null);
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

  // the bell and its popup live in their own overlay webview. the shell tracks what the *rail*
  // shows: the DM inbox, and the unread badge on each server icon.
  const refreshFeed = useCallback(async () => {
    const [nextDms, counts] = await Promise.all([api.listDms(), api.unreadCounts()]);

    setDms(nextDms);
    setUnread(counts);
  }, []);

  useEffect(() => {
    serversRef.current = registry.servers;
  }, [registry.servers]);

  useEffect(() => {
    activeIdRef.current = activeId;
  }, [activeId]);

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
      try {
        const isReady = await api.prepareServer(id);

        if (isReady) {
          await showServerNow(id);

          return;
        }

        // the shell keeps the area rather than handing it to a client that would only show its own
        // connecting state
        await api.showShell();

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

  const openPanel = useCallback(async (next: Exclude<Panel, 'server'>) => {
    try {
      await api.showShell();

      // the user has gone somewhere else, so a server that connects now must not pull them back
      pendingRef.current = null;

      setPanel(next);
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, []);

  /// reopens the inbox, restoring the conversation it was last showing
  const openDmPanel = useCallback(async () => {
    try {
      await api.showShell();

      pendingRef.current = null;

      setPanel('dms');
      setError(null);

      if (lastDm) {
        await api.openDm(lastDm.entryId, lastDm.name);
      }
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, [lastDm]);

  /// closing the inbox is the one place a conversation view is torn down, so the second client does
  /// not stay connected once the user is done with DMs
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

  useEffect(() => {
    const boot = async () => {
      try {
        const next = await refresh();

        await refreshFeed();

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
  }, [openServer, refresh, refreshFeed]);

  const refreshVoice = useCallback(async () => {
    setVoice(await api.voiceStatus().catch(() => null));
  }, []);

  // the core drains each live page on a timer and says so here, rather than the shell polling
  useEffect(() => {
    const pending = listen(FEED_EVENT, () => {
      refreshFeed();
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, [refreshFeed]);

  // the core drives this rather than the shell polling: two of the three states are not events, a
  // server goes from connecting to offline by nothing happening for long enough
  useEffect(() => {
    const pending = listen<Record<string, ServerStatus>>(STATUS_EVENT, (event) => {
      setStatuses(event.payload);
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, []);

  // a call can start, move or end on any server, including one nobody is looking at
  useEffect(() => {
    refreshVoice();

    const pending = listen(VOICE_EVENT, () => {
      refreshVoice();
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, [refreshVoice]);

  // the swap out of Shiver's cached view and into the live client
  useEffect(() => {
    const pending = listen<{ entryId: string }>(SERVER_READY_EVENT, (event) => {
      if (pendingRef.current !== event.payload.entryId) return;

      showServerNow(event.payload.entryId).catch(() => undefined);
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, [showServerNow]);


  /// opens the conversation beside Shiver's DM list, drawn by that server's own client
  const handleOpenDm = useCallback(
    async (entryId: string, name: string, channelId: number) => {
      try {
        await api.openDm(entryId, name);

        setActiveId(entryId);
        setOpenedDm(`${entryId}:${channelId}`);
        setLastDm({ entryId, name });
        setDmError(null);
        setError(null);
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    []
  );

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

        if (activeId !== id) return;

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
    [activeId, openServer, refresh, refreshFeed]
  );

  const handleRenameFolder = useCallback(
    async (id: string, name: string) => {
      try {
        await api.renameFolder(id, name);
        await refresh();

        setRenamingFolder(null);
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

      setSigningIn(null);
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
      } catch (cause) {
        setError(errorMessage(cause));
      }
    },
    [refresh]
  );

  // a conversation the page could not open. the split still shows whatever was there before, so
  // Shiver must stop marking the new one as open rather than quietly disagreeing with the screen.
  useEffect(() => {
    const pending = listen<{ name: string }>(DM_FAILED_EVENT, (event) => {
      setOpenedDm(null);
      // reported in the DM sidebar, not the content area: the server's webview covers that
      setDmError(`Could not open the conversation with ${event.payload.name}.`);
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, []);

  /** A server's page is asking for credentials: collect them here, so Shiver can renew the session. */
  useEffect(() => {
    const pending = listen<{ entryId: string }>(SIGNED_OUT_EVENT, (event) => {
      const { entryId } = event.payload;

      // only for the server being looked at or waited on: Shiver must not drag the user off
      // whatever they are doing because a background server lost its session
      if (activeIdRef.current !== entryId && pendingRef.current !== entryId) return;

      setSigningIn(entryId);
      openPanel('signin').catch(() => undefined);
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, [openPanel]);

  // Clicking a message in the bell's feed. The feed is drawn in the popup's own webview, which
  // cannot switch servers, so it asks and this does it — through the same `openServer` everything
  // else goes through, so a server that is still coming up gets the same cover it always does.
  useEffect(() => {
    const pending = listen<{
      entryId: string;
      channelId: number | null;
      isDm: boolean;
      author: string;
    }>(OPEN_MESSAGE_EVENT, async (event) => {
      const { entryId, channelId, isDm, author } = event.payload;

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
        if (channelId !== null) {
          await api.selectChannel(entryId, channelId);
        }
      } catch (cause) {
        setError(errorMessage(cause));
      }
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, [openPanel, openServer]);

  // the rail's context menu is a native os menu, so its result comes back as an event
  useEffect(() => {
    const pending = listen<{ action: string; entryId: string }>(MENU_EVENT, async (event) => {
      const { action, entryId } = event.payload;

      if (action === 'open') {
        await openServer(entryId);

        return;
      }

      if (action === 'remove') {
        await handleRemove(entryId);

        return;
      }

      if (action === 'markread') {
        await api.markServerRead(entryId).catch(() => undefined);
        await refreshFeed();

        return;
      }

      if (action === 'signin') {
        setSigningIn(entryId);
        await openPanel('signin');

        return;
      }

      if (action === 'forgetpw') {
        await api.forgetPassword(entryId);

        return;
      }

      // two actions rather than a toggle, so this does not have to know which way the server is
      // currently set — the menu knew, and said so by which item it built
      if (action === 'anysize' || action === 'normalsize') {
        await api.setAcceptAnySize(entryId, action === 'anysize').catch(() => undefined);

        return;
      }

      if (action === 'logout') {
        await api.logOutServer(entryId).catch(() => undefined);
        await refresh();

        return;
      }

      if (action === 'refresh') {
        await api.refreshServerInfo(entryId).catch(() => undefined);
        await refresh();

        return;
      }

      if (action === 'unfolder') {
        await api.setServerFolder(entryId, null).catch(() => undefined);
        await refresh();

        return;
      }

      if (action === 'delete-folder') {
        await api.deleteFolder(entryId).catch(() => undefined);
        await refresh();

        return;
      }

      if (action === 'rename-folder') {
        // `entryId` carries the folder id for folder actions
        setRenamingFolder(entryId);
        await openPanel('folder');
      }
    });

    return () => {
      pending.then((unsubscribe) => unsubscribe()).catch(() => undefined);
    };
  }, [handleRemove, openPanel, openServer, refresh, refreshFeed]);

  useEffect(() => {
    applyTheme(registry.settings);
  }, [registry.settings]);

  const showWelcome = ready && panel === 'server' && !activeId && !error;

  return (
    <div className="app">
      <ServerRail
        servers={servers}
        folders={registry.folders}
        activeId={activeId}
        unread={unread}
        statuses={statuses}
        settingsOpen={panel === 'settings'}
        dmsOpen={panel === 'dms'}
        voice={voice}
        onSelect={openServer}
        onAdd={() => openPanel('add')}
        onOpenSettings={() => openPanel('settings')}
        onOpenDms={openDmPanel}
        onRefresh={refresh}
      />

      <div className="main">
        <div className="content">
          {/* over the top of whatever is on screen, so it is seen on the launch it was found */}
          <UpdateNotice />

          {error ? (
            <div className="panel">
              <p className="error">{error}</p>
            </div>
          ) : null}

          {panel === 'add' ? (
            <AddServerPanel
              onAdded={handleAdded}
              onCancel={closePanel}
              canCancel={servers.length > 0}
            />
          ) : null}

          {panel === 'settings' ? (
            <SettingsPanel
              settings={registry.settings}
              onSave={handleSettingsSaved}
              onClose={closePanel}
            />
          ) : null}

          {panel === 'signin' ? (
            <SignInPanel
              server={registry.servers.find((server) => server.id === signingIn)}
              onSignIn={handleSignIn}
              onCancel={closePanel}
            />
          ) : null}

          {panel === 'folder' ? (
            <RenameFolderPanel
              folder={registry.folders.find((folder) => folder.id === renamingFolder)}
              onSave={handleRenameFolder}
              onCancel={closePanel}
            />
          ) : null}

          {panel === 'connecting' && connecting ? (
            <ConnectingPanel
              serverName={connecting.serverName}
              failed={connecting.failed}
              onRetry={() => openServer(connecting.entryId)}
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
