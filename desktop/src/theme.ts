import { DEFAULT_RAIL_COLOR, DEFAULT_THEME_COLOR, type Settings } from './types';

/** Relative luminance, to decide whether text on a colour should be dark or light. */
const isLight = (hex: string) => {
  const value = hex.replace('#', '');

  if (value.length !== 6) return true;

  const channel = (offset: number) => {
    const srgb = parseInt(value.slice(offset, offset + 2), 16) / 255;

    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };

  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4) > 0.35;
};

/**
 * The text colour Shiver picks for a background when the user has not chosen one.
 *
 * Exported so the settings screen's swatch can show it: a picker has to show some colour, and
 * showing this one means opening it changes nothing until the user actually moves it.
 */
export const automaticTextColor = (themeColor: string) =>
  isLight(themeColor) ? '#171717' : '#fafafa';

/**
 * Paints Shiver's own chrome in the user's colours.
 *
 * Every Shiver webview calls this, not just the shell: the bell and the notification popup are
 * separate pages and would otherwise keep the defaults while the rail changed colour.
 *
 * On the default theme the rail keeps Sharkord's own separate sidebar shade rather than flattening
 * the window into one tone.
 */
export const applyTheme = (settings: Settings) => {
  const { themeColor, accentColor, textColor } = settings;
  const root = document.documentElement;
  const isDefaultTheme = themeColor.toLowerCase() === DEFAULT_THEME_COLOR;

  root.style.setProperty('--shiver-bg', themeColor);
  root.style.setProperty('--shiver-rail', isDefaultTheme ? DEFAULT_RAIL_COLOR : themeColor);
  root.style.setProperty('--shiver-accent', accentColor);
  root.style.setProperty('--shiver-on-accent', isLight(accentColor) ? '#171717' : '#fafafa');

  // surfaces are derived from the chosen background so buttons, rows and menus keep their contrast
  // whatever colour the user picks, rather than staying on the default greys
  root.style.setProperty(
    '--shiver-surface',
    `color-mix(in srgb, ${themeColor} 84%, ${isLight(themeColor) ? '#000' : '#fff'})`
  );
  root.style.setProperty(
    '--shiver-surface-hover',
    `color-mix(in srgb, ${themeColor} 72%, ${isLight(themeColor) ? '#000' : '#fff'})`
  );
  // Their own colour where they have chosen one, and otherwise whichever of black or white the
  // background can carry. Not stored as a colour when it is automatic: a colour would stay put
  // after the background moved out from under it.
  const text = textColor ?? automaticTextColor(themeColor);

  root.style.setProperty('--shiver-text', text);
  root.style.setProperty('--shiver-text-dim', `color-mix(in srgb, ${text} 65%, ${themeColor})`);
};
