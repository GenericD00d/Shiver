/** Colour helpers shared by both clients' UI and both bridges. */

/** Whether dark text reads better than light on `hex` (relative luminance). Non-`#rrggbb` is "light". */
export const isLightColor = (hex: string) => {
  const value = hex.replace('#', '');

  if (value.length !== 6) return true;

  const channel = (offset: number) => {
    const srgb = parseInt(value.slice(offset, offset + 2), 16) / 255;

    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  };

  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4) > 0.35;
};

/** The text colour used on a background when the user has not chosen one. */
export const automaticTextColor = (background: string) => (isLightColor(background) ? '#171717' : '#fafafa');

/** Mixes `color` towards black (light colours) or white (dark ones) by `100 - percent`. */
export const lift = (color: string, percent: number) =>
  `color-mix(in srgb, ${color} ${percent}%, ${isLightColor(color) ? '#000' : '#fff'})`;
