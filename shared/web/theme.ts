/** Paints Shiver's own pages in the user's colours, as `--shiver-*` custom properties. */

import { automaticTextColor, DEFAULT_RAIL_COLOR, DEFAULT_THEME_COLOR, lift } from './colors';

export { automaticTextColor };

type Colors = { themeColor: string; accentColor: string; textColor?: string | null };

export const applyTheme = ({ themeColor, accentColor, textColor }: Colors) => {
  const root = document.documentElement.style;
  const text = textColor ?? automaticTextColor(themeColor);

  root.setProperty('--shiver-bg', themeColor);
  // on the default theme the rail keeps Sharkord's sidebar shade, so the window is two tones
  root.setProperty('--shiver-rail', themeColor.toLowerCase() === DEFAULT_THEME_COLOR ? DEFAULT_RAIL_COLOR : themeColor);
  root.setProperty('--shiver-accent', accentColor);
  root.setProperty('--shiver-on-accent', automaticTextColor(accentColor));
  root.setProperty('--shiver-surface', lift(themeColor, 84));
  root.setProperty('--shiver-surface-hover', lift(themeColor, 72));
  root.setProperty('--shiver-surface-dim', lift(themeColor, 92));
  root.setProperty('--shiver-text', text);
  root.setProperty('--shiver-text-dim', `color-mix(in srgb, ${text} 65%, ${themeColor})`);
};
