/** Page features both bridges install in a Sharkord client. */

import { defineHook, ensureStyle, onDomSettled } from './dom';
import { callPlugin, waitForPlugin } from './plugin';
import {
  FILE_CARD,
  MEMBER_ITEM,
  MENTION_CHIP,
  MESSAGE_WRAPPER,
  REPLY_AUTHOR,
  SETTINGS_TRIGGER,
  type SharkordRole,
  type SharkordUser,
  watchStore
} from './sharkord';

declare global {
  interface Window {
    /** changes how loud this page's own sounds are without reloading it */
    __SHIVER_SET_SOUND_VOLUME__?: (percent: number) => void;
    /** turns the minimised attachment cards on or off without reloading the page */
    __SHIVER_SET_ATTACHMENT_CARDS__?: (minimised: boolean) => void;
  }
}

const toLevel = (percent: number) => (Number.isFinite(percent) ? Math.max(0, percent) / 100 : 1);

/** `AudioNode.prototype.connect`, narrowed to the node-to-node overload. */
type Connect = (this: AudioNode, destination: AudioNode, output?: number, input?: number) => AudioNode;

/**
 * Routes everything connected to an `AudioContext`'s destination through one gain node per
 * context, so the volume setting can exceed 100% (a gain has no ceiling; media elements do). Only
 * Sharkord's synthesised tones pass through here: call audio plays through media elements. A
 * second install only changes the level. Must run before anything else that captures `connect`.
 */
export function installSoundVolume(percent: number) {
  if (window.__SHIVER_SET_SOUND_VOLUME__) {
    window.__SHIVER_SET_SOUND_VOLUME__(percent);

    return;
  }

  let level = toLevel(percent);

  const masters = new Set<GainNode>();
  const perContext = new WeakMap<BaseAudioContext, GainNode>();
  const connect = AudioNode.prototype.connect as Connect;

  const masterFor = (context: BaseAudioContext) => {
    let gain = perContext.get(context);

    if (!gain) {
      gain = context.createGain();
      gain.gain.value = level;
      connect.call(gain, context.destination);
      perContext.set(context, gain);
      masters.add(gain);
    }

    return gain;
  };

  AudioNode.prototype.connect = function patched(
    this: AudioNode,
    destination: AudioNode | AudioParam,
    output?: number,
    input?: number
  ) {
    // the master has one input, so the page's input index is dropped
    if (destination instanceof AudioDestinationNode) return connect.call(this, masterFor(destination.context), output);

    return connect.call(this, destination as AudioNode, output, input);
  } as AudioNode['connect'];

  defineHook('__SHIVER_SET_SOUND_VOLUME__', (next: number) => {
    level = toLevel(next);

    for (const gain of masters) gain.gain.value = level;
  });
}

const ATTACHMENT_MINIMISED = 'shiver-attachment-min';

/**
 * Shrinks a file card to its icon (and delete button) when the page is already showing the file
 * inline, e.g. the card under a picture. Name and size move to the tooltip. Cards for anything not
 * shown inline stay whole. A second install only changes the setting.
 */
export function installAttachmentCards(minimised: boolean) {
  if (window.__SHIVER_SET_ATTACHMENT_CARDS__) {
    window.__SHIVER_SET_ATTACHMENT_CARDS__(minimised);

    return;
  }

  ensureStyle('shiver-attachment-cards').textContent = `
a.${ATTACHMENT_MINIMISED} { max-width: none; width: fit-content; gap: 4px; padding: 2px;
  border-color: transparent; background: none; box-shadow: none; opacity: 0.55; }
a.${ATTACHMENT_MINIMISED}:hover { opacity: 1; }
a.${ATTACHMENT_MINIMISED} > [class~="bg-muted"] { padding: 2px; background: none; }
a.${ATTACHMENT_MINIMISED} > [class~="flex-1"] { display: none; }
`;

  let on = minimised;

  const paint = () => {
    const cards = document.querySelectorAll<HTMLAnchorElement>(FILE_CARD);

    if (cards.length === 0) return;

    const shown = new Set<string>();

    if (on) {
      for (const media of document.querySelectorAll<HTMLImageElement | HTMLMediaElement>(
        'img[src], video[src], audio[src], source[src]'
      )) {
        shown.add(media.src);
      }
    }

    for (const card of cards) {
      const duplicate = shown.has(card.href);

      if (duplicate === card.classList.contains(ATTACHMENT_MINIMISED)) continue;

      card.classList.toggle(ATTACHMENT_MINIMISED, duplicate);

      if (duplicate) {
        card.title = [...card.querySelectorAll('span')].map((span) => span.textContent?.trim()).join(' · ');
      } else {
        card.removeAttribute('title');
      }
    }
  };

  paint();
  onDomSettled(paint);

  defineHook('__SHIVER_SET_ATTACHMENT_CARDS__', (next: boolean) => {
    on = next;
    paint();
  });
}

/**
 * Makes voice controls agree with the member list: screen share purple and camera blue
 * everywhere (Sharkord draws your own share button blue and camera green, the reverse of the
 * member list). Both the sidebar voice bar (/15, text-400) and the call's bottom bar (/20,
 * text-500) are covered; the hover class keeps other green chips (voice debug, live ring) green.
 */
export function installVoiceColors() {
  const pair = (selector: string, bg: string, hoverBg: string, color: string, hoverColor: string) => `
${selector} { background-color: ${bg} !important; color: ${color} !important; }
${selector}:hover { background-color: ${hoverBg} !important; color: ${hoverColor} !important; }`;

  ensureStyle('shiver-voice-colors').textContent = [
    pair('[class~="bg-blue-500/15"]', 'rgb(168 85 247 / 15%)', 'rgb(168 85 247 / 25%)', 'rgb(192 132 252)', 'rgb(216 180 254)'),
    pair(
      '[class~="bg-green-500/15"][class~="hover:bg-green-500/25"]',
      'rgb(59 130 246 / 15%)',
      'rgb(59 130 246 / 25%)',
      'rgb(96 165 250)',
      'rgb(147 197 253)'
    ),
    pair('[class~="bg-blue-500/20"]', 'rgb(168 85 247 / 20%)', 'rgb(168 85 247 / 30%)', 'rgb(168 85 247)', 'rgb(168 85 247)'),
    pair(
      '[class~="bg-green-500/20"][class~="hover:bg-green-500/30"]',
      'rgb(59 130 246 / 20%)',
      'rgb(59 130 246 / 30%)',
      'rgb(59 130 246)',
      'rgb(59 130 246)'
    )
  ].join('\n');
}

/**
 * A user's colour: their first coloured non-default role in the server's list order (Sharkord has
 * no role hierarchy), else the default role's. `#ffffff` is Sharkord's "no colour".
 */
function roleColor(user: SharkordUser, roles: SharkordRole[]) {
  let fallback: string | null = null;

  for (const role of roles) {
    if (!user.roleIds?.includes(role.id)) continue;

    const color = role.color?.trim();

    if (!color || ['#ffffff', '#fff'].includes(color.toLowerCase())) continue;
    if (!role.isDefault) return color;

    fallback ??= color;
  }

  return fallback;
}

/**
 * Draws usernames (message headers, member list, reply previews, mentions other than your own) in
 * their role colour. Names are matched by text, since the DOM carries no user ids. Only nodes
 * Shiver coloured are ever reset, and a node already showing the right colour is left untouched.
 */
export function installRoleColors() {
  let colors = new Map<string, string>();
  let ownName = '';
  let signature = '';
  /** whether any name has been coloured, so a server without role colours costs no DOM walks */
  let painted = false;

  const paintNode = (node: HTMLElement | null | undefined, name = node?.textContent?.trim() ?? '') => {
    if (!node) return;

    const color = colors.get(name) ?? '';

    if ((node.dataset.shiverRole ?? '') === color) return;

    node.style.color = color;

    if (color) node.dataset.shiverRole = color;
    else delete node.dataset.shiverRole;
  };

  const paint = () => {
    if (colors.size === 0 && !painted) return;

    painted = colors.size > 0;

    for (const wrapper of document.querySelectorAll<HTMLElement>(MESSAGE_WRAPPER)) {
      paintNode(wrapper.parentElement?.previousElementSibling?.querySelector<HTMLElement>(':scope > span'));
    }

    for (const row of document.querySelectorAll<HTMLElement>(MEMBER_ITEM)) {
      paintNode(row.querySelector<HTMLElement>(':scope > span'));
    }

    for (const name of document.querySelectorAll<HTMLElement>(REPLY_AUTHOR)) paintNode(name);

    for (const chip of document.querySelectorAll<HTMLElement>(MENTION_CHIP)) {
      const name = chip.textContent?.trim().replace(/^@/, '') ?? '';

      // your own mention keeps Sharkord's "you were pinged" colour
      if (name && name !== ownName) paintNode(chip, name);
    }
  };

  watchStore((state) => {
    const users = state.users ?? [];
    const next = new Map<string, string>();

    for (const user of users) {
      const color = roleColor(user, state.roles ?? []);

      if (color) next.set(user.name, color);
    }

    ownName = users.find((user) => user.id === state.ownUserId)?.name ?? '';

    const stamp = `${ownName}|${[...next].join('|')}`;

    if (stamp === signature) return;

    signature = stamp;
    colors = next;
    paint();
  });

  onDomSettled(paint);
}

const STATUS_BUTTON_ID = 'shiver-status-button';
const STATUS_POPOVER_ID = 'shiver-status-popover';
const STATUS_BACKDROP_ID = 'shiver-status-backdrop';

/** lucide's `smile` */
const SMILE_ICON =
  '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" ' +
  'stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
  '<circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/>' +
  '<line x1="9" x2="9.01" y1="9" y2="9"/><line x1="15" x2="15.01" y1="9" y2="9"/></svg>';

const statusStyles = (touch: boolean) => `
#${STATUS_BACKDROP_ID} { position: fixed; inset: 0; z-index: 2147483646; background: rgb(0 0 0 / 45%); }
#${STATUS_POPOVER_ID} {
  position: fixed; z-index: 2147483647; box-sizing: border-box; display: flex; flex-direction: column;
  width: ${touch ? 'min(320px, calc(100vw - 32px))' : '280px'}; padding: ${touch ? 14 : 12}px;
  gap: ${touch ? 10 : 8}px; border-radius: ${touch ? 12 : 10}px;
  background: var(--popover, #1f1f1f); color: var(--popover-foreground, #fafafa);
  border: 1px solid var(--border, rgb(255 255 255 / 12%)); box-shadow: 0 12px 32px rgb(0 0 0 / 50%);
  font: 500 ${touch ? 14 : 13}px/1.4 system-ui, -apple-system, "Segoe UI", sans-serif;
}
#${STATUS_POPOVER_ID} label { color: var(--muted-foreground, #a1a1a1); font-size: 12px; }
#${STATUS_POPOVER_ID} input {
  width: 100%; box-sizing: border-box; padding: ${touch ? '10px' : '7px 9px'}; border-radius: ${touch ? 8 : 7}px;
  background: var(--input, rgb(255 255 255 / 6%)); color: inherit; font: inherit; outline: none;
  border: 1px solid var(--border, rgb(255 255 255 / 14%));
}
#${STATUS_POPOVER_ID} p { margin: 0; color: #f87171; font-size: 12px; }
#${STATUS_POPOVER_ID} div { display: flex; gap: ${touch ? 10 : 8}px; justify-content: flex-end; }
#${STATUS_POPOVER_ID} button {
  padding: ${touch ? '8px 16px' : '6px 12px'}; ${touch ? 'min-height: 40px;' : 'cursor: pointer;'}
  border-radius: ${touch ? 8 : 7}px; border: none; font: inherit;
  background: var(--primary, #e5e5e5); color: var(--primary-foreground, #171717);
}
#${STATUS_POPOVER_ID} button.ghost { background: transparent; color: var(--muted-foreground, #a1a1a1); }
`;

/**
 * A "set your status" button beside Sharkord's settings gear, on servers with the companion
 * plugin. `touch` centres the popover on the visual viewport (above the keyboard) over a backdrop;
 * otherwise it opens above the button and closes on an outside click.
 */
export function installStatusButton(touch: boolean) {
  ensureStyle('shiver-status-style').textContent = statusStyles(touch);

  let release: (() => void) | null = null;

  const close = () => {
    release?.();
    release = null;
    document.getElementById(STATUS_POPOVER_ID)?.remove();
    document.getElementById(STATUS_BACKDROP_ID)?.remove();
  };

  const open = (anchor: Element) => {
    if (document.getElementById(STATUS_POPOVER_ID)) return close();

    const host = document.createElement('div');
    const label = document.createElement('label');
    const input = document.createElement('input');
    const note = document.createElement('p');
    const row = document.createElement('div');
    const clear = document.createElement('button');
    const save = document.createElement('button');

    host.id = STATUS_POPOVER_ID;
    label.textContent = 'Your status — shown beside your name while you are online';
    input.maxLength = 100;
    input.placeholder = 'What are you up to?';
    input.enterKeyHint = 'done';
    note.hidden = true;
    clear.textContent = 'Clear';
    clear.className = 'ghost';
    save.textContent = 'Save';
    row.append(clear, save);
    host.append(label, input, note, row);

    if (touch) {
      const backdrop = document.createElement('div');
      const viewport = window.visualViewport;

      backdrop.id = STATUS_BACKDROP_ID;
      // on the backdrop, so the tap never reaches (and closes) the drawer underneath
      backdrop.addEventListener('pointerdown', (event) => {
        event.preventDefault();
        close();
      });
      document.body.append(backdrop, host);

      // the keyboard shrinks the visual viewport after the popover opens, so this follows it
      const place = () => {
        const size = host.getBoundingClientRect();
        const width = viewport?.width ?? window.innerWidth;
        const height = viewport?.height ?? window.innerHeight;

        host.style.left = `${(viewport?.offsetLeft ?? 0) + Math.max(8, (width - size.width) / 2)}px`;
        host.style.top = `${(viewport?.offsetTop ?? 0) + Math.max(8, (height - size.height) / 2)}px`;
      };

      place();
      viewport?.addEventListener('resize', place);
      viewport?.addEventListener('scroll', place);
      release = () => {
        viewport?.removeEventListener('resize', place);
        viewport?.removeEventListener('scroll', place);
      };
    } else {
      document.body.append(host);

      const box = anchor.getBoundingClientRect();
      const size = host.getBoundingClientRect();

      host.style.left = `${Math.max(8, Math.min(box.left, window.innerWidth - size.width - 8))}px`;
      host.style.top = `${Math.max(8, box.top - size.height - 8)}px`;

      const outside = (event: MouseEvent) => {
        if (!(event.target instanceof Node && host.contains(event.target))) close();
      };

      // after this click has finished dispatching
      window.setTimeout(() => document.addEventListener('mousedown', outside, true), 0);
      release = () => document.removeEventListener('mousedown', outside, true);
    }

    input.focus();

    let typed = false;

    input.addEventListener('input', () => {
      typed = true;
    });

    // shown before the current status arrives; typing while it is in flight wins
    void callPlugin('getOwnStatus').then((current) => {
      if (!host.isConnected || typed) return;

      input.value = typeof current?.status === 'string' ? current.status : '';
      input.select();
    });

    const commit = async (value: string) => {
      save.disabled = clear.disabled = true;
      note.hidden = true;

      // the relay answers with the stored status, so success is confirmed rather than assumed
      const result = await callPlugin('setStatus', { status: value });

      if (typeof result?.status === 'string') return close();

      note.textContent = 'Could not save that. The Shiver plugin may no longer be installed here.';
      note.hidden = false;
      save.disabled = clear.disabled = false;
    };

    save.addEventListener('click', () => void commit(input.value));
    clear.addEventListener('click', () => void commit(''));
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') void commit(input.value);
      if (event.key === 'Escape') close();
    });
  };

  // Sharkord re-renders the panel often (and mobile's drawer unmounts it), so this re-adds it
  const add = () => {
    if (document.getElementById(STATUS_BUTTON_ID)) return;

    const gear = document.querySelector(SETTINGS_TRIGGER);

    if (!gear) return;

    const button = document.createElement('button');

    button.id = STATUS_BUTTON_ID;
    button.type = 'button';
    button.className = gear.className;
    button.title = 'Set your status';
    button.setAttribute('aria-label', 'Set your status');
    button.innerHTML = SMILE_ICON;
    button.addEventListener('click', (event) => {
      event.preventDefault();
      event.stopPropagation();
      open(button);
    });

    gear.parentElement?.insertBefore(button, gear);
  };

  void waitForPlugin().then((present) => {
    if (!present) return;

    add();
    onDomSettled(add);
  });
}
