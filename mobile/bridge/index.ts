/**
 * The Shiver bridge (mobile): evaluated in a Sharkord page once it has loaded. It is handed this
 * entry's own config and nothing about the user's other servers. The page has no IPC: back and a
 * swipe past the drawer navigate to Shiver's own page, where the rail is, and the core polls the
 * hooks in `types.ts`. The session was served from memory since document start (`document-start.ts`).
 */

import { defineHook, isTopFrame, onDomSettled } from '../../shared/web/bridge/dom';
import { installAttachmentCards, installRoleColors, installSoundVolume, installStatusButton, installVoiceColors } from '../../shared/web/bridge/features';
import { callPlugin, pushMutesToPlugin, storeReadFloor, syncMutesWithPlugin, waitForPlugin } from '../../shared/web/bridge/plugin';
import { installMuteStyles, paintMuted } from '../../shared/web/bridge/sharkord';
import { applyPageTheme } from '../../shared/web/bridge/theme';
import { installSessionShim, takeSeedFromLocation } from '../../shared/web/session';
import { goHome, installHomeSwipe, setHome } from './home';
import { installAutoReconnect } from './reconnect';
import {
  closeChannelMenu,
  installChannelMenu,
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
  installRoleColors();
  installReactionNames();
  installReturnMakesALine();
  installStatusButton(true);
  installAutoReconnect(shiver.session, shiver.serverName, shiver.entryId);

  const muted = new Set(shiver.muted);
  const paint = () => paintMuted(muted);

  defineHook('__SHIVER_MUTED__', () => [...muted]);
  installChannelMenu({
    has: (channelId) => muted.has(channelId),
    toggle: (channelId) => {
      if (!muted.delete(channelId)) muted.add(channelId);

      paint();
      pushMutesToPlugin([...muted]);
    }
  });
  paint();
  onDomSettled(paint);

  void syncMutesWithPlugin([...muted]).then((merged) => {
    if (!merged) return;

    muted.clear();

    for (const id of merged) muted.add(id);

    paint();
  });

  const { pushEndpoint, retiredPushEndpoints } = shiver;

  if (pushEndpoint || retiredPushEndpoints.length) {
    void waitForPlugin().then((present) => {
      if (!present) return;

      // the plugin checks the endpoint before storing it against this user
      if (pushEndpoint) void callPlugin('setPushEndpoint', { endpoint: pushEndpoint });
      for (const endpoint of retiredPushEndpoints) void callPlugin('clearPushEndpoint', { endpoint });
    });
  }

  // arriving at a server is what moves its shared unread floor
  if (shiver.readFloor) void storeReadFloor(shiver.readFloor);

  defineHook('__SHIVER_BACK__', () => closeChannelMenu() || goHome());
  installHomeSwipe();

  if (shiver.openDmUser) openConversation(shiver.openDmUser);
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
