/** Sharkord's voice controls, read and pressed, and the lock that refuses a second call. */

import { CHANNEL_ITEM, rowName, type SharkordState, SIDEBAR } from '../../shared/web/bridge/sharkord';
import type { VoiceAction, VoiceSnapshot } from './types';

/** Whether `element` contains lucide icon `name` (whole class token: `mic` is a prefix of `mic-off`). */
export function hasLucideIcon(element: Element, name: string) {
  return [...element.querySelectorAll('svg')].some(
    (icon) => icon.classList.contains(`lucide-${name}`) || icon.classList.contains(`lucide-${name}-icon`)
  );
}

/**
 * Sharkord's own voice controls. Mic and deafen are the pair of buttons sharing a parent (the same
 * icons appear per participant in a voice channel's user list, but not as sibling buttons).
 */
function findVoiceButtons(): Record<VoiceAction, HTMLButtonElement | null> {
  const scope = document.querySelector(SIDEBAR) ?? document.body;
  const buttons = [...scope.querySelectorAll<HTMLButtonElement>('button')];
  const isMic = (button: Element) => hasLucideIcon(button, 'mic') || hasLucideIcon(button, 'mic-off');
  const isSound = (button: Element) => hasLucideIcon(button, 'headphones') || hasLucideIcon(button, 'headphone-off');
  const soundBeside = (button: Element) =>
    ([...(button.parentElement?.children ?? [])].find((sibling) => sibling !== button && isSound(sibling)) as HTMLButtonElement | undefined) ?? null;

  const mic = buttons.find((button) => isMic(button) && soundBeside(button)) ?? null;

  return {
    mic,
    sound: mic ? soundBeside(mic) : null,
    leave: buttons.find((button) => hasLucideIcon(button, 'phone-off')) ?? null
  };
}

export function readVoice(state: SharkordState): VoiceSnapshot | null {
  const channelId = state.currentVoiceChannelId;

  if (typeof channelId !== 'number') return null;

  const { mic, sound } = findVoiceButtons();

  // missing controls read as unmuted: claiming a live mic is muted would be worse
  return {
    channelId,
    channelName: (state.channels ?? []).find((channel) => channel.id === channelId)?.name ?? null,
    micMuted: mic ? hasLucideIcon(mic, 'mic-off') : false,
    soundMuted: sound ? hasLucideIcon(sound, 'headphone-off') : false,
    micLocked: mic?.disabled ?? false
  };
}

/** Clicks one of Sharkord's voice controls; unknown actions and disabled controls do nothing. */
export function runVoiceAction(action: VoiceAction) {
  const buttons = findVoiceButtons();
  const button = Object.hasOwn(buttons, action) ? buttons[action] : null;

  if (button && !button.disabled) button.click();
}

let voiceLocked = false;

/** Another server took or let go of the call. */
export function setVoiceLocked(locked: boolean) {
  voiceLocked = locked;
}

let voiceNoticeTimer: number | null = null;

/**
 * Refuses a click on a voice channel while another server holds the call (capture phase on the
 * document, before React sees it), so the second join never happens. Rows carry only a name, so
 * voice rows are recognised through the store, and a text row of the same name by its icon.
 */
export function installVoiceLock(initial: boolean, getState: () => SharkordState) {
  voiceLocked = initial;

  document.addEventListener(
    'click',
    (event) => {
      if (!voiceLocked) return;

      const row = (event.target as Element | null)?.closest?.(CHANNEL_ITEM);

      if (!row) return;

      const name = rowName(row);

      // a text row draws Sharkord's `#` icon: one named like a voice channel is not one
      if (hasLucideIcon(row, 'hash')) return;
      if (!(getState().channels ?? []).some((channel) => channel.type === 'VOICE' && channel.name === name)) return;

      event.preventDefault();
      event.stopImmediatePropagation();
      showVoiceNotice();
    },
    true
  );
}

/** Says, where the user clicked, why the voice channel did not open (without naming the other server). */
function showVoiceNotice() {
  const id = 'shiver-voice-notice';
  const notice = document.getElementById(id) ?? document.body.appendChild(document.createElement('div'));

  notice.id = id;
  notice.textContent = 'You are already in a voice channel on another server in Shiver.';
  notice.setAttribute('role', 'status');
  notice.setAttribute(
    'style',
    'position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%); z-index: 2147483647; ' +
      'padding: 10px 16px; border-radius: 8px; font-size: 13px; pointer-events: none; ' +
      'background: var(--popover, #171717); color: var(--popover-foreground, #fafafa); ' +
      'border: 1px solid var(--border, rgba(250,250,250,0.12)); box-shadow: 0 8px 24px rgba(0,0,0,0.35)'
  );

  if (voiceNoticeTimer !== null) window.clearTimeout(voiceNoticeTimer);

  voiceNoticeTimer = window.setTimeout(() => {
    notice.remove();
    voiceNoticeTimer = null;
  }, 4000);
}
