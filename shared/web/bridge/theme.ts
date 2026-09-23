/** Repaints a server's own client in the user's colours (both bridges). */

import { automaticTextColor, lift } from '../colors';
import { ensureStyle } from './dom';

export type ShiverTheme = {
  themeColor: string;
  accentColor: string;
  /** the user's text colour, or null to derive it from the background */
  textColor: string | null;
};

/**
 * The two rules the theme is written into, `:root` and `.dark` (Sharkord renders under a
 * hard-coded `dark` class, so the override has to match it on specificity).
 */
function themeRules(): CSSStyleRule[] {
  const sheet = ensureStyle('shiver-theme').sheet;

  if (!sheet) return [];

  if (sheet.cssRules.length === 0) {
    sheet.insertRule(':root {}', 0);
    sheet.insertRule('.dark {}', 1);
  }

  return [...sheet.cssRules].filter((rule): rule is CSSStyleRule => rule instanceof CSSStyleRule);
}

/**
 * Sets Sharkord's theme variables from the user's colours, deriving every surface from the
 * background with `color-mix`; `null` restores Sharkord's own. Written through the CSSOM, so a
 * value cannot break out of its declaration.
 */
export function applyPageTheme(theme: ShiverTheme | null) {
  const rules = themeRules();

  if (!theme) {
    for (const rule of rules) {
      while (rule.style.length > 0) rule.style.removeProperty(rule.style.item(0));
    }

    return;
  }

  const { themeColor, accentColor } = theme;
  const foreground = theme.textColor ?? automaticTextColor(themeColor);
  const border = `color-mix(in srgb, ${foreground} 12%, transparent)`;

  const variables: Record<string, string> = {
    '--background': themeColor,
    '--foreground': foreground,
    '--sidebar': lift(themeColor, 90),
    '--sidebar-foreground': foreground,
    '--card': lift(themeColor, 90),
    '--card-foreground': foreground,
    '--popover': lift(themeColor, 88),
    '--popover-foreground': foreground,
    '--muted-foreground': `color-mix(in srgb, ${foreground} 65%, ${themeColor})`,
    '--muted': lift(themeColor, 84),
    '--secondary': lift(themeColor, 84),
    '--accent': lift(themeColor, 80),
    '--accent-foreground': foreground,
    '--sidebar-accent': lift(themeColor, 80),
    '--input': lift(themeColor, 78),
    '--border': border,
    '--sidebar-border': border,
    '--primary': accentColor,
    '--primary-foreground': automaticTextColor(accentColor),
    '--sidebar-primary': accentColor,
    '--ring': accentColor,
    '--sidebar-ring': accentColor
  };

  for (const rule of rules) {
    for (const [name, value] of Object.entries(variables)) rule.style.setProperty(name, value, 'important');
  }
}
