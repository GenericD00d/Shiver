/**
 * Shiver's notification ping: Sharkord's MESSAGE_RECEIVED tone reproduced (600 Hz sine, 0.05 gain
 * through a x2 master, 50 ms decay), scaled by the volume setting, which may exceed 100%.
 */

const MASTER_GAIN = 2;

let context: AudioContext | null = null;

const getContext = () => {
  const AudioContextCtor =
    window.AudioContext ??
    (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;

  if (!AudioContextCtor) return null;

  if (!context || context.state === 'closed') {
    context = new AudioContextCtor();
  }

  return context;
};

/**
 * Plays the ping. Silent rather than throwing when the webview has not had a user gesture yet and
 * the audio context cannot start, which is the same rule a browser applies to Sharkord itself.
 */
export const playNotificationSound = async (percent = 100) => {
  // clamped to the same ceiling as `MAX_SOUND_VOLUME` in model.rs, and never negative
  const level = Number.isFinite(percent) ? Math.min(Math.max(percent, 0), 250) / 100 : 1;

  try {
    const ctx = getContext();

    if (!ctx) return;

    if (ctx.state === 'suspended') {
      await ctx.resume();
    }

    if (ctx.state !== 'running') return;

    const now = ctx.currentTime;
    const oscillator = ctx.createOscillator();
    const gain = ctx.createGain();

    oscillator.type = 'sine';
    oscillator.frequency.setValueAtTime(600, now);

    gain.gain.setValueAtTime(0.05 * MASTER_GAIN * level, now);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.05);

    oscillator.connect(gain).connect(ctx.destination);
    oscillator.start(now);
    oscillator.stop(now + 0.05);
  } catch {
    // a device with no audio output is not worth reporting
  }
};
