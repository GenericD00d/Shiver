/** Setting defaults and limits shared by both clients. Must match `shared/shiver-core/src/model.rs`. */

/** Sharkord's own dark theme. */
export const DEFAULT_THEME_COLOR = '#0a0a0a';
export const DEFAULT_ACCENT_COLOR = '#e5e5e5';
/** Sharkord's sidebar shade, kept for the rail on the default theme (the web clients' alone). */
export const DEFAULT_RAIL_COLOR = '#171717';
/** Loudest the sound slider goes (the core and the bridge clamp to it). */
export const MAX_SOUND_VOLUME = 250;

/** The colours the user picks, as a set (`Colors` in the core). */
export type Colors = {
  themeColor: string;
  accentColor: string;
  /** null takes it from the background */
  textColor: string | null;
  /** null is the accent, made readable on the background */
  accentTextColor: string | null;
};

export type ColorPreset = Colors & { name: string };

/** Most presets the user can save, and the longest name (the core keeps to the same). */
export const MAX_COLOR_PRESETS = 20;
export const MAX_PRESET_NAME = 32;

/** Presets every Shiver has, ahead of the user's own. */
export const BUILT_IN_PRESETS: readonly ColorPreset[] = [
  {
    name: 'Sharkord',
    themeColor: DEFAULT_THEME_COLOR,
    accentColor: DEFAULT_ACCENT_COLOR,
    textColor: null,
    accentTextColor: null
  },
  { name: 'Cord-like', themeColor: '#18181a', accentColor: '#1f1f25', textColor: null, accentTextColor: null }
];

/** Whether two sets are the same colours (case aside). */
export const sameColors = (a: Colors, b: Colors) =>
  a.themeColor.toLowerCase() === b.themeColor.toLowerCase() &&
  a.accentColor.toLowerCase() === b.accentColor.toLowerCase() &&
  (a.textColor ?? '').toLowerCase() === (b.textColor ?? '').toLowerCase() &&
  (a.accentTextColor ?? '').toLowerCase() === (b.accentTextColor ?? '').toLowerCase();
