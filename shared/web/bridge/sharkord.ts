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

type SharkordStore = {
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
const UNREAD_COUNT = '[data-testid="unread-count"]';
export const MESSAGE_ITEM = '[data-testid="message-item"]';
export const MEMBER_ITEM = '[data-testid="member-item"]';
export const SETTINGS_TRIGGER = '[data-testid="user-settings-trigger"]';
export const COMPOSE_EDITOR = '[data-testid="message-compose-editor"]';
/** where Sharkord portals its full-screen picture */
export const IMAGE_VIEWER = '#imagePortal';
/** Sharkord's "Connection lost" dialog while it retries, over the (blurred, inert) channel */
export const RECONNECTING_OVERLAY = '[role="alertdialog"][aria-live="polite"][class~="backdrop-blur-sm"]';
/** Sharkord's voice-channel chat and thread panels while open (a closed one is `w-0`) */
export const SIDE_PANEL = '.bg-card[class~="hidden"][class~="lg:flex"]:not(.w-0)';
/** below Tailwind's `lg`, where Sharkord hides its top bar and those panels */
export const NARROW = '(width < 64rem)';
export const CONNECT_FORM = '[data-testid="connect-form"]';
export const SERVER_VIEW = '[data-testid="server-view"]';
/** each message's wrapper; its parent's previous sibling is the author header */
export const MESSAGE_WRAPPER = '[id^="message-"]';
/** the author's name in an inline reply's preview line */
export const REPLY_AUTHOR = '[class~="max-w-40"][class~="truncate"][class~="font-medium"]';
/** `<span class="mention">@Name</span>` inside a message */
export const MENTION_CHIP = 'span.mention';
/** a reaction pill the user is part of (Sharkord marks it with only a 1px border) */
const REACTED_PILL = '[class~="h-9"][class~="border-border"]';
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

/* ── reading the store ── */

/** One of this server's DMs, as Shiver lists it. */
export type DmChannel = { channelId: number; name: string; iconUrl: string | null; lastMessageAt: number | null };

/** The other participant of a DM channel, which the server names `DM - <userA>:<userB>`. */
export function dmPartnerId(channel: SharkordChannel, ownUserId: number | undefined) {
  const match = channel.name.match(/^DM - (\d+):(\d+)$/);

  if (!match) return null;

  const [a, b] = [Number(match[1]), Number(match[2])];

  return ownUserId !== undefined && a === ownUserId ? b : a;
}

export function findDmChannelIdByUserName(state: SharkordState, name: string) {
  const user = (state.users ?? []).find((candidate) => candidate.name === name);

  if (!user) return null;

  return (state.channels ?? []).find((channel) => channel.isDm && dmPartnerId(channel, state.ownUserId) === user.id)?.id ?? null;
}

export type NotificationTarget = { channelId: number | null; channelName: string | null; author: string; isDm: boolean };

const DM_TITLE = ' (DM)';
const IN_CHANNEL = ' in #';

/**
 * Who a notification is from and which channel it is for, from its title alone: Sharkord titles
 * them `Author (DM)` or `Author in #channel`. A name may itself contain ` in #`, so the split taken
 * is the one naming a channel that exists. Matched by name: `selectedChannelId` is exactly the
 * channel a notification is not for.
 */
export function notificationTarget(title: string, state: SharkordState): NotificationTarget {
  if (title.endsWith(DM_TITLE)) {
    const author = title.slice(0, -DM_TITLE.length).trim();

    return { channelId: findDmChannelIdByUserName(state, author), channelName: null, author, isDm: true };
  }

  const channels = (state.channels ?? []).filter((channel) => !channel.isDm);
  let first: NotificationTarget | null = null;

  for (let at = title.indexOf(IN_CHANNEL); at !== -1; at = title.indexOf(IN_CHANNEL, at + 1)) {
    const author = title.slice(0, at).trim();
    const channelName = title.slice(at + IN_CHANNEL.length);
    const channel = channels.find((candidate) => candidate.name === channelName);

    if (channel) return { channelId: channel.id, channelName, author, isDm: false };

    first ??= { channelId: null, channelName, author, isDm: false };
  }

  return first ?? { channelId: null, channelName: null, author: title.trim(), isDm: false };
}

/**
 * A public url for one of the server's files, carrying its access token when it has one. A name
 * with an empty or dot segment is refused, since the url would leave `/public/`.
 */
export function fileUrl(origin: string, file: SharkordFile | null | undefined) {
  const segments = file?.name.split('/');

  if (!file || !segments || segments.some((segment) => segment === '' || segment === '.' || segment === '..')) return null;

  try {
    const url = new URL(`/public/${segments.map(encodeURIComponent).join('/')}`, origin);

    if (file._accessToken) {
      url.searchParams.set('accessToken', file._accessToken);

      if (file._accessTokenExpiresAt) url.searchParams.set('expires', String(file._accessTokenExpiresAt));
    }

    return url.toString();
  } catch {
    return null;
  }
}

/**
 * This server's DMs, newest first (by `lastSeen`, per channel), then by name, so the list does not
 * reorder under the pointer. A DM whose partner is not in the store yet is skipped rather than
 * listed under its raw channel name.
 */
export function readDms(origin: string, state: SharkordState, lastSeen: ReadonlyMap<number, number>): DmChannel[] {
  const users = new Map((state.users ?? []).map((user) => [user.id, user]));
  const list: DmChannel[] = [];

  for (const channel of state.channels ?? []) {
    if (!channel.isDm) continue;

    const partner = users.get(dmPartnerId(channel, state.ownUserId) ?? -1);

    if (partner) {
      list.push({ channelId: channel.id, name: partner.name, iconUrl: fileUrl(origin, partner.avatar), lastMessageAt: lastSeen.get(channel.id) ?? null });
    }
  }

  return list.sort((a, b) => (b.lastMessageAt ?? 0) - (a.lastMessageAt ?? 0) || a.name.localeCompare(b.name));
}

/* ── reading the page ── */

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

/** an attribute rather than a class: React rewrites a row's classes, never an attribute it did not set */
const MUTED = 'data-shiver-muted';
const SHIVER_MENU_ITEM = 'shiver-menu-item';

/**
 * Styles muted channel rows (dimmed, unread pill hidden), Shiver's item in Sharkord's menus, and
 * reaction pills the user is part of (tinted in the accent).
 */
export function installMuteStyles() {
  ensureStyle('shiver-mute-style').textContent = `
[${MUTED}] { opacity: 0.45; }
[${MUTED}] ${UNREAD_COUNT} { display: none !important; }
.${SHIVER_MENU_ITEM}:hover, .${SHIVER_MENU_ITEM}:focus { background-color: var(--accent); color: var(--accent-foreground); }
${REACTED_PILL} {
  background-color: color-mix(in srgb, var(--primary) 22%, transparent) !important;
  border-color: var(--primary) !important;
}
`;
}

/**
 * Marks the sidebar rows of muted channels: `rows`, or every row. Rows carry no channel id, so
 * they are matched by name (two channels sharing a name dim together; the mute is keyed on the id).
 */
export function paintMuted(muted: ReadonlySet<number>, rows: Iterable<Element> = document.querySelectorAll(CHANNEL_ITEM)) {
  const list = [...rows];

  if (!list.length) return;

  const names = new Set(
    (sharkordStore()?.getState().channels ?? []).filter((channel) => muted.has(channel.id)).map((channel) => channel.name)
  );

  for (const row of list) row.toggleAttribute(MUTED, names.has(rowName(row)));
}

/**
 * Adds "Mute/Unmute in Shiver" to one of Sharkord's own (Radix) menus, styled like its siblings.
 * Replaces Shiver's items left from an earlier open, since Radix reuses its menu.
 */
export function addMuteItem(menu: HTMLElement, isMuted: boolean, toggle: () => void) {
  for (const stale of menu.querySelectorAll(`.${SHIVER_MENU_ITEM}`)) stale.remove();

  addMenuItem(menu, isMuted ? 'Unmute in Shiver' : 'Mute in Shiver', toggle);
}

const pressEscape = () => document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));

/** Closes the topmost of Sharkord's open dialogs (its settings among them) as Escape does; true when one was open. */
export function closeDialog() {
  if (!document.querySelector('[role="dialog"]:not([data-state="closed"])')) return false;

  pressEscape();

  return true;
}

/** Appends one of Shiver's items to a Sharkord menu, styled as Sharkord's own. */
export function addMenuItem(menu: HTMLElement, label: string, run: () => void) {
  const sibling = menu.querySelector<HTMLElement>(`[role="menuitem"]:not(.${SHIVER_MENU_ITEM})`);
  const item = document.createElement('div');

  item.setAttribute('role', 'menuitem');
  item.tabIndex = -1;
  item.className = `${sibling?.className ?? ''} ${SHIVER_MENU_ITEM}`.trim();
  item.textContent = label;

  item.addEventListener('mouseenter', () => item.focus());
  item.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    run();
    // lets Sharkord close its menu as it would for any item
    pressEscape();
  });

  menu.appendChild(item);
}
