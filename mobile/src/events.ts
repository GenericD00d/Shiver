import { listen } from '@tauri-apps/api/event';

import { coreEventHook } from '../../shared/web/events';

/** Calls the latest handler with each payload of an event for as long as the component is mounted. */
export const useCoreEvent = coreEventHook(listen);

/** What the core tells Shiver's own page (`useCoreEvent` in shared/web/events.ts). */
export const EVENTS = {
  /** the core's own check found a newer version: its number */
  update: 'shiver://update',
  /** an endpoint arrived or a registration was refused, so the push screen can redraw */
  push: 'shiver://push',
  /** the unread counts moved: every server's count */
  unread: 'shiver://inbox',
  /** a server's name or logo changed: the registry and logos are worth reading again */
  servers: 'shiver://servers',
  /** a server opens only after a wait, since Sharkord allows only a few joins a minute */
  joinWait: 'shiver://join-wait',
  /** a `sharkord://` link arrived while Shiver's page is up: `take_server_link` has it */
  serverLink: 'shiver://server-link'
} as const;
