/** Paints Shiver's own pages in the user's colours, as `--shiver-*` custom properties. */

import { automaticTextColor, lift, readableAccent } from './colors';
import { type Colors, DEFAULT_RAIL_COLOR, DEFAULT_THEME_COLOR } from './settings';

type Theme = Pick<Colors, 'themeColor' | 'accentColor'> &
  Partial<Pick<Colors, 'textColor' | 'accentTextColor' | 'sidebarColor'>>;

/** The colour for text and icons in the accent: the user's, else the accent kept readable. */
export const accentTextColor = ({ themeColor, accentColor, textColor, accentTextColor }: Theme) =>
  accentTextColor ?? readableAccent(accentColor, themeColor, textColor ?? automaticTextColor(themeColor));

export const applyTheme = (theme: Theme) => {
  const { themeColor, accentColor } = theme;
  const root = document.documentElement.style;
  const text = theme.textColor ?? automaticTextColor(themeColor);

  root.setProperty('--shiver-bg', themeColor);
  // the rail is the user's sidebar colour, as Discord's server list matches its channel list; else
  // on the default theme it keeps Sharkord's sidebar shade, so the window is two tones
  root.setProperty(
    '--shiver-rail',
    theme.sidebarColor ?? (themeColor.toLowerCase() === DEFAULT_THEME_COLOR ? DEFAULT_RAIL_COLOR : themeColor)
  );
  root.setProperty('--shiver-accent', accentColor);
  root.setProperty('--shiver-on-accent', automaticTextColor(accentColor));
  root.setProperty('--shiver-accent-text', accentTextColor(theme));
  root.setProperty('--shiver-surface', lift(themeColor, 84));
  root.setProperty('--shiver-surface-hover', lift(themeColor, 72));
  root.setProperty('--shiver-border', `color-mix(in srgb, ${text} 10%, transparent)`);
  root.setProperty('--shiver-text', text);
  root.setProperty('--shiver-text-dim', `color-mix(in srgb, ${text} 65%, ${themeColor})`);
};
