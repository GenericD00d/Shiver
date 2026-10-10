/**
 * The Shiver bridge (mobile): evaluated in a Sharkord page once it has loaded. It is handed this
 * entry's own config and nothing about the user's other servers. The page has no IPC: back and a
 * swipe past the drawer ask for the rail with a navigation the core turns into the rail drawn over
 * this page (`home.ts`), and the core polls the hooks in `types.ts`. The session was served from memory since document start (`document-start.ts`).
 */

import { closeChannelMenu } from '../../shared/web/bridge/channel-menu';
import { defineHook, isTopFrame, onDomSettled, touched } from '../../shared/web/bridge/dom';
import {
  closeSidePanel,
  installAttachmentCards,
  installSidePanels,
  installSoundVolume,
  installVoiceColors
} from '../../shared/web/bridge/features';
import { callPlugin, pushMutesToPlugin, storeReadFloor, syncMutesWithPlugin, waitForPlugin } from '../../shared/web/bridge/plugin';
import { CHANNEL_ITEM, installMuteStyles, paintMuted, readDms, sharkordStore } from '../../shared/web/bridge/sharkord';
import { applyPageTheme } from '../../shared/web/bridge/theme';
import { installSessionShim, takeSeedFromLocation } from '../../shared/web/session';
import { installRailSwipe, openRail, setHome } from './home';
import { installAutoReconnect, installQuietReconnect } from './reconnect';
import {
  installImageZoom,
  installLongPress,
  installReactionNames,
  installReturnMakesALine,
  installTouchStyles,
  openConversation
} from './touch';
import type { ShiverConfig } from './types';

function install(shiver: ShiverConfig) {
  // Android can report one load twice; everything here is per document
  if (window.__SHIVER_MOBILE_INSTALLED__) return;

  defineHook('__SHIVER_MOBILE_INSTALLED__', true);

  seedSession(shiver.session);
  setHome(shiver.home);

  applyPageTheme(shiver.theme);
  installMuteStyles();
  installTouchStyles();
  installSoundVolume(shiver.soundVolume);
  installAttachmentCards(shiver.minimiseAttachments);
  installVoiceColors();
  installSidePanels();
  installReactionNames();
  installImageZoom();
  installReturnMakesALine();
  installAutoReconnect(shiver.session, shiver.entryId);
  installQuietReconnect();

  const muted = new Set(shiver.muted);
  const paint = () => paintMuted(muted);

  defineHook('__SHIVER_MUTED__', () => [...muted]);
  installDmList(shiver.origin);
  installLongPress({
    has: (channelId) => muted.has(channelId),
    toggle: (channelId) => {
      if (!muted.delete(channelId)) muted.add(channelId);

      paint();
      pushMutesToPlugin([...muted]);
    }
  });
  paint();
  onDomSettled((changed) => paintMuted(muted, touched(changed, CHANNEL_ITEM)));

  void syncMutesWithPlugin([...muted]).then((merged) => {
    if (!merged) return;

    muted.clear();

    for (const id of merged) muted.add(id);

    paint();
  });

  const { pushEndpoint, retiredPushEndpoints } = shiver;
  // endpoints the plugin confirmed clearing, until the core reads them: only those are forgotten
  const cleared: string[] = [];

  defineHook('__SHIVER_CLEARED__', () => cleared.splice(0));

  if (pushEndpoint || retiredPushEndpoints.length) {
    void waitForPlugin().then((present) => {
      if (!present) return;

      // the plugin checks the endpoint before storing it against this user
      if (pushEndpoint) void callPlugin('setPushEndpoint', { endpoint: pushEndpoint });

      for (const endpoint of retiredPushEndpoints) {
        void callPlugin('clearPushEndpoint', { endpoint }).then((result) => {
          if (result) cleared.push(endpoint);
        });
      }
    });
  }

  // arriving at a server is what moves its shared unread floor
  if (shiver.readFloor) void storeReadFloor(shiver.readFloor);

  defineHook('__SHIVER_BACK__', () => closeChannelMenu() || closeSidePanel() || openRail());
  installRailSwipe();

  if (shiver.openDmUser) openConversation(shiver.openDmUser);
}

/**
 * This server's conversations, for Shiver's inbox, which otherwise knows them only from the core's
 * own connection: there is none while this page is up, and none at all to a server without a
 * stored session. Read by the core with the mutes; handed over only when they changed, and only
 * once the page has joined (before that the store is empty, which is not "no conversations"). Their
 * times come separately, from `document-start.ts`.
 */
function installDmList(origin: string) {
  let reported = '';

  defineHook('__SHIVER_DMS__', () => {
    const state = sharkordStore()?.getState();

    if (typeof state?.ownUserId !== 'number') return null;

    const list = readDms(origin, state, new Map()).map(({ channelId, name, iconUrl }) => ({ channelId, name, iconUrl }));
    // avatar urls carry expiring tokens, so they are left out of the comparison
    const signature = JSON.stringify(list.map(({ channelId, name }) => [channelId, name]));

    if (signature === reported) return null;

    reported = signature;

    return list;
  });
}

/**
 * Makes sure the session is served from memory. Normally the document-start script already did
 * that from the URL fragment; on a WebView without document-start scripts (or after a reload that
 * dropped the fragment) the fragment is stripped here and the shim installed late, which still
 * beats Sharkord's auto-login (it waits for plugins to load). A shim that stepped aside (token
 * refused, or the user signed in themselves) is left alone.
 */
function seedSession(session: string | null) {
  takeSeedFromLocation();

  if (session && !window.__SHIVER_SESSION_SHIM__) installSessionShim(session);
}

// read and removed from the page at once; only the top frame installs
const config = window.__SHIVER__;

delete window.__SHIVER__;

if (config && isTopFrame()) {
  try {
    install(config);
  } catch (error) {
    console.error('[shiver] bridge failed to install', error);
  }
}
