/** Page features both bridges install in a Sharkord client. */

import { MAX_SOUND_VOLUME } from '../settings';
import { defineHook, ensureStyle, onDomSettled, touched } from './dom';
import { FILE_CARD, MESSAGE_WRAPPER, NARROW, SIDE_PANEL } from './sharkord';

declare global {
  interface Window {
    /** changes how loud this page's own sounds are without reloading it */
    __SHIVER_SET_SOUND_VOLUME__?: (percent: number) => void;
    /** turns the minimised attachment cards on or off without reloading the page */
    __SHIVER_SET_ATTACHMENT_CARDS__?: (minimised: boolean) => void;
  }
}

const toLevel = (percent: number) => (Number.isFinite(percent) ? Math.min(Math.max(0, percent), MAX_SOUND_VOLUME) / 100 : 1);

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

const ATTACHMENT_MINIMISED = 'data-shiver-minimised';

/**
 * Shrinks a file card to its icon (and delete button) when its message already shows the file
 * inline, e.g. the card under a picture. Name and size move to the tooltip. Cards for anything not
 * shown inline stay whole. A second install only changes the setting.
 */
export function installAttachmentCards(minimised: boolean) {
  if (window.__SHIVER_SET_ATTACHMENT_CARDS__) {
    window.__SHIVER_SET_ATTACHMENT_CARDS__(minimised);

    return;
  }

  ensureStyle('shiver-attachment-cards').textContent = `
a[${ATTACHMENT_MINIMISED}] { max-width: none; width: fit-content; gap: 4px; padding: 2px;
  border-color: transparent; background: none; box-shadow: none; opacity: 0.55; }
a[${ATTACHMENT_MINIMISED}]:hover { opacity: 1; }
a[${ATTACHMENT_MINIMISED}] > [class~="bg-muted"] { padding: 2px; background: none; }
a[${ATTACHMENT_MINIMISED}] > [class~="flex-1"] { display: none; }
`;

  let on = minimised;

  const shownIn = (card: HTMLAnchorElement) =>
    [...(card.closest(MESSAGE_WRAPPER) ?? document).querySelectorAll<HTMLImageElement | HTMLMediaElement>(
      'img[src], video[src], audio[src], source[src]'
    )].some((media) => media.src === card.href);

  const paint = (cards: Iterable<HTMLAnchorElement> = document.querySelectorAll<HTMLAnchorElement>(FILE_CARD)) => {
    for (const card of cards) {
      const duplicate = on && shownIn(card);

      if (duplicate === card.hasAttribute(ATTACHMENT_MINIMISED)) continue;

      card.toggleAttribute(ATTACHMENT_MINIMISED, duplicate);

      if (duplicate) {
        card.title = [...card.querySelectorAll('span')].map((span) => span.textContent?.trim()).join(' · ');
      } else {
        card.removeAttribute('title');
      }
    }
  };

  paint();
  // cards that changed, and those in messages that did (a picture can arrive after its card)
  onDomSettled((changed) => {
    const cards = touched<HTMLAnchorElement>(changed, FILE_CARD);

    for (const message of touched(changed, MESSAGE_WRAPPER)) {
      for (const card of message.querySelectorAll<HTMLAnchorElement>(FILE_CARD)) cards.add(card);
    }

    paint(cards);
  });

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
 * Sharkord hides its voice-channel chat and thread panels on a narrow page, so opening one there
 * did nothing; it covers the page instead (its own × closes it), under Sharkord's dialogs and menus.
 */
export function installSidePanels() {
  ensureStyle('shiver-side-panels').textContent = `@media ${NARROW} {
${SIDE_PANEL} { display: flex !important; position: fixed !important; inset: 0 !important; width: auto !important; border: 0 !important; z-index: 45; }
}`;
}

/** Closes the side panel covering a narrow page, the topmost if two are; true when there was one. */
export function closeSidePanel() {
  const panel = window.matchMedia(NARROW).matches ? [...document.querySelectorAll(SIDE_PANEL)].pop() : undefined;
  const close = panel?.querySelector('button svg.lucide-x')?.closest('button');

  close?.click();

  return !!close;
}
