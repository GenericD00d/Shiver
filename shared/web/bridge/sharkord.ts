/**
 * What the bridges know about Sharkord's client: its plugin store, the test ids and class
 * combinations they match against, and the small operations both clients perform through it.
 */

import { ensureStyle } from './dom';

export type SharkordFile = { name: string; _accessToken?: string; _accessTokenExpiresAt?: number };

export type SharkordChannel = {
  id: number;
  name: string;
  isDm?: boolean;
  /** `TEXT` or `VOICE` */
  type?: string;
};

export type SharkordUser = { id: number; name: string; avatar?: SharkordFile | null; roleIds?: number[] };

export type SharkordRole = { id: number; name: string; color?: string; isDefault?: boolean };

export type SharkordState = {
  channels?: SharkordChannel[];
  users?: SharkordUser[];
  roles?: SharkordRole[];
  ownUserId?: number;
  selectedChannelId?: number;
  currentVoiceChannelId?: number | null;
};

export type SharkordStore = {
  getState: () => SharkordState;
  subscribe?: (listener: () => void) => () => void;
  /** plugin-facing actions; these take the plugin id as an argument, unlike `executePluginAction` */
  actions?: {
    selectChannel?: (channelId: number) => void;
    getUserData?: (pluginId: string) => Promise<Record<string, unknown> | null>;
    setUserData?: (pluginId: string, data: Record<string, unknown>) => Promise<void>;
  };
};

declare global {
  interface Window {
    __SHARKORD_STORE__?: SharkordStore;
  }
}

/* Sharkord's test ids, and class combinations where there is none (attribute selectors, so no
   escaping is needed; nothing else in the client carries the same combination). */
export const SIDEBAR = '[data-testid="left-sidebar"]';
export const CHANNEL_ITEM = '[data-testid="channel-item"]';
export const DM_ITEM = '[data-testid="dm-item"]';
export const DM_TOGGLE = '[data-testid="dm-toggle"]';
export const UNREAD_COUNT = '[data-testid="unread-count"]';
export const MESSAGE_ITEM = '[data-testid="message-item"]';
export const MEMBER_ITEM = '[data-testid="member-item"]';
export const SETTINGS_TRIGGER = '[data-testid="user-settings-trigger"]';
export const CONNECT_FORM = '[data-testid="connect-form"]';
export const SERVER_VIEW = '[data-testid="server-view"]';
/** each message's wrapper; its parent's previous sibling is the author header */
export const MESSAGE_WRAPPER = '[id^="message-"]';
/** the author's name in an inline reply's preview line */
export const REPLY_AUTHOR = '[class~="max-w-40"][class~="truncate"][class~="font-medium"]';
/** `<span class="mention">@Name</span>` inside a message */
export const MENTION_CHIP = 'span.mention';
/** a reaction pill the user is part of (Sharkord marks it with only a 1px border) */
export const REACTED_PILL = '[class~="h-9"][class~="border-border"]';
/** a file card: an anchor to the file with its icon, name, size and sometimes a delete button */
export const FILE_CARD = 'a[class~="max-w-sm"][class~="rounded-lg"][class~="border-border"]';

/** The id Shiver's companion plugin installs under (`SHIVER_PLUGIN_ID` in sharkord-client). */
export const SHIVER_PLUGIN_ID = 'shiver';

export const sharkordStore = () => window.__SHARKORD_STORE__;

/**
 * Calls `onChange` with the store's state now and on every change. The store is published by the
 * client's entry point, which may not have run yet, so this polls until it appears.
 */
export function watchStore(onChange: (state: SharkordState) => void) {
  const start = () => {
    const store = sharkordStore();

    if (!store?.subscribe) return false;

    const read = () => {
      try {
        onChange(store.getState());
      } catch {
        // a read during teardown
      }
    };

    store.subscribe(read);
    read();

    return true;
  };

  if (start()) return;

  const timer = window.setInterval(() => {
    if (start()) window.clearInterval(timer);
  }, 500);
}

/**
 * The channel name a sidebar row shows. Not `textContent`, which includes the unread pill
 * ("general3"): the name's own span, else the text minus the trailing count.
 */
export function rowName(row: Element): string {
  const named = row.querySelector<HTMLElement>('span.flex-1, span.truncate');

  if (named) return named.textContent?.trim() ?? '';

  const text = row.textContent?.trim() ?? '';
  const count = row.querySelector(UNREAD_COUNT)?.textContent?.trim() ?? '';

  return (count && text.endsWith(count) ? text.slice(0, -count.length) : text).trim();
}

/** The (non-DM) channel a sidebar row stands for, matched by name since rows carry no id. */
export function channelOfRow(row: Element): SharkordChannel | null {
  const name = rowName(row);
  const channels = sharkordStore()?.getState().channels ?? [];

  return (name && channels.find((channel) => !channel.isDm && channel.name === name)) || null;
}

/**
 * Marks every text channel read by selecting each (Sharkord marks the selected channel read, and a
 * channel with nothing unread costs no request), then returns to the channel the user was on.
 */
export function markAllChannelsRead() {
  const store = sharkordStore();
  const selectChannel = store?.actions?.selectChannel;

  if (!store || !selectChannel) return;

  const { selectedChannelId, channels = [] } = store.getState();

  for (const channel of channels) {
    if (!channel.isDm && channel.type === 'TEXT') selectChannel(channel.id);
  }

  if (typeof selectedChannelId === 'number') selectChannel(selectedChannelId);
}

export const MUTE_CLASS = 'shiver-muted-channel';
export const SHIVER_MENU_ITEM = 'shiver-menu-item';

/**
 * Styles muted channel rows (dimmed, unread pill hidden), Shiver's item in Sharkord's menus, and
 * reaction pills the user is part of (tinted in the accent).
 */
export function installMuteStyles() {
  ensureStyle('shiver-mute-style').textContent = `
.${MUTE_CLASS} { opacity: 0.45; }
.${MUTE_CLASS} ${UNREAD_COUNT} { display: none !important; }
.${SHIVER_MENU_ITEM}:hover, .${SHIVER_MENU_ITEM}:focus { background-color: var(--accent); color: var(--accent-foreground); }
${REACTED_PILL} {
  background-color: color-mix(in srgb, var(--primary) 22%, transparent) !important;
  border-color: var(--primary) !important;
}
`;
}

/**
 * Marks the sidebar rows of muted channels. Rows carry no channel id, so they are matched by name
 * (two channels sharing a name dim together; the mute itself is keyed on the id).
 */
export function paintMuted(muted: ReadonlySet<number>) {
  const names = new Set(
    (sharkordStore()?.getState().channels ?? []).filter((channel) => muted.has(channel.id)).map((channel) => channel.name)
  );

  for (const row of document.querySelectorAll<HTMLElement>(CHANNEL_ITEM)) {
    row.classList.toggle(MUTE_CLASS, names.has(rowName(row)));
  }
}

/**
 * Adds "Mute/Unmute in Shiver" to one of Sharkord's own (Radix) menus, styled like its siblings.
 * Replaces an item left from an earlier open, since Radix reuses its menu.
 */
export function addMuteItem(menu: HTMLElement, isMuted: boolean, toggle: () => void) {
  menu.querySelector(`.${SHIVER_MENU_ITEM}`)?.remove();

  const sibling = menu.querySelector<HTMLElement>('[role="menuitem"]');
  const item = document.createElement('div');

  item.setAttribute('role', 'menuitem');
  item.tabIndex = -1;
  item.className = `${sibling?.className ?? ''} ${SHIVER_MENU_ITEM}`.trim();
  item.textContent = isMuted ? 'Unmute in Shiver' : 'Mute in Shiver';

  item.addEventListener('mouseenter', () => item.focus());
  item.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    toggle();
    // lets Sharkord close its menu as it would for any item
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  });

  menu.appendChild(item);
}
