import { listen } from '@tauri-apps/api/event';

import { coreEventHook } from '../../shared/web/events';

/** Calls the latest handler with each payload of an event for as long as the component is mounted. */
export const useCoreEvent = coreEventHook(listen);

/** What the core tells Shiver's own webviews. */
export const EVENTS = {
  feed: 'shiver://feed',
  settings: 'shiver://settings',
  popup: 'shiver://popup',
  menu: 'shiver://server-menu',
  openMessage: 'shiver://open-message',
  /** a `sharkord://` link arrived: `take_server_link` has what it offers */
  serverLink: 'shiver://server-link',
  dmFailed: 'shiver://dm-failed',
  serverReady: 'shiver://server-ready',
  voice: 'shiver://voice',
  status: 'shiver://status',
  /** servers whose client speaks another version of Sharkord's desktop api (`Mismatch`) */
  mismatch: 'shiver://mismatch',
  signedOut: 'shiver://signed-out',
  update: 'shiver://update',
  /** a rail shortcut: `{ server: n }` (1-based) or `{ step: ±1 }` */
  shortcut: 'shiver://shortcut'
} as const;
