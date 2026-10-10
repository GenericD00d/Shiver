/** Conversations shown beside Shiver's DM list, and channels opened on the core's word. */

import { ensureStyle } from '../../shared/web/bridge/dom';
import {
  closeDialog,
  DM_ITEM,
  DM_TOGGLE,
  dmRowName,
  findDmChannelIdByUserName,
  isDmRowFor,
  type SharkordState,
  sharkordStore,
  SIDEBAR
} from '../../shared/web/bridge/sharkord';

const OPEN_DM_TIMEOUT_MS = 25_000;

/** Escapes one request may press, so a dialog that ignores Escape cannot hold it up */
const MAX_ESCAPES = 3;

/**
 * A retry loop's first step: closes an open dialog, since one (Sharkord's settings) would stay over
 * the conversation or channel being opened. True while it closed one.
 */
function dialogCloser() {
  let escapes = 0;

  return () => escapes < MAX_ESCAPES && closeDialog() && ++escapes > 0;
}

/**
 * The DM open in Sharkord's DM view. Not in the plugin store (`selectedDmChannelId` lives in
 * another slice), so it is read off the highlighted row, by the same name notifications resolve by.
 */
export function openDmChannelId(state: SharkordState) {
  const row = [...document.querySelectorAll<HTMLElement>(DM_ITEM)].find((candidate) => candidate.classList.contains('bg-accent'));

  if (!row) return null;

  let name = dmRowName(row);

  if (name === null) {
    // markup changed: the longest user name in the row, so "Ana" never beats "Ana B"
    const text = row.textContent ?? '';

    for (const user of state.users ?? []) {
      if (user.name && text.includes(user.name) && user.name.length > (name?.length ?? 0)) name = user.name;
    }
  }

  return name ? findDmChannelIdByUserName(state, name) : null;
}

/** whether the page left Sharkord's channels to show a conversation, so leaving goes back to them */
let conversationLeftChannels: boolean | null = null;

/**
 * Shows the conversation with `name` beside Shiver's DM list, hiding Sharkord's sidebar (Shiver's
 * list stands in for it); `null` shows the sidebar again and, if the page was on its channels
 * before, goes back to them.
 */
export function showConversation(name: string | null) {
  if (name === null) {
    if (conversationLeftChannels === null) return;

    ensureStyle('shiver-dm-mode').textContent = '';

    if (conversationLeftChannels && document.querySelector(DM_ITEM)) document.querySelector<HTMLElement>(DM_TOGGLE)?.click();

    conversationLeftChannels = null;

    return;
  }

  conversationLeftChannels ??= !document.querySelector(DM_ITEM);
  ensureStyle('shiver-dm-mode').textContent = `${SIDEBAR} { display: none !important; }`;
  openDirectMessage(name);
}

let openDmTimer: number | null = null;

/** set by `install`, so a failed open reaches the next drain */
let reportOpenDmFailure: () => void = () => undefined;

/** Who hears that a conversation could not be opened. */
export function onOpenDmFailure(report: () => void) {
  reportOpenDmFailure = report;
}

/**
 * Opens the DM with `name` by clicking Sharkord's own controls: the DM toggle (pressed at most
 * once, only while no DM rows exist, since it is a toggle) and then the row. Retries until a
 * deadline long enough to outlast a cold page connecting; a newer request replaces an older one.
 */
function openDirectMessage(name: string) {
  if (openDmTimer !== null) window.clearInterval(openDmTimer);

  openDmTimer = null;

  let toggled = false;
  const deadline = Date.now() + OPEN_DM_TIMEOUT_MS;
  const closing = dialogCloser();

  const attempt = () => {
    if (closing()) return false;

    const rows = [...document.querySelectorAll<HTMLElement>(DM_ITEM)];

    if (rows.length === 0) {
      const toggle = toggled ? null : document.querySelector<HTMLElement>(DM_TOGGLE);

      toggle?.click();
      toggled ||= !!toggle;
    } else {
      const row = rows.find((candidate) => isDmRowFor(candidate, name));

      if (row) {
        row.click();

        return true;
      }
    }

    if (Date.now() <= deadline) return false;

    reportOpenDmFailure();

    return true;
  };

  if (attempt()) return;

  openDmTimer = window.setInterval(() => {
    if (!attempt()) return;

    if (openDmTimer !== null) window.clearInterval(openDmTimer);

    openDmTimer = null;
  }, 150);
}

/** Selects a channel (for a clicked notification) once the client can, until a deadline. */
export function selectChannelWhenReady(channelId: number) {
  const deadline = Date.now() + OPEN_DM_TIMEOUT_MS;
  const closing = dialogCloser();

  const attempt = () => {
    if (closing()) return false;

    const store = sharkordStore();
    const selectChannel = store?.actions?.selectChannel;

    if (selectChannel && (store.getState().channels ?? []).some((channel) => channel.id === channelId)) {
      selectChannel(channelId);

      return true;
    }

    return Date.now() > deadline;
  };

  if (attempt()) return;

  const timer = window.setInterval(() => {
    if (attempt()) window.clearInterval(timer);
  }, 250);
}
