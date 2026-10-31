/** Colour helpers shared by both clients' UI and both bridges. */

/** Whether dark text reads better than light on `hex` (relative luminance). Non-`#rrggbb` is "light". */
const isLightColor = (hex: string) => {
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

const channels = (hex: string) => {
  const value = hex.replace('#', '');

  return value.length === 6 ? [0, 2, 4].map((offset) => parseInt(value.slice(offset, offset + 2), 16)) : null;
};

const luminance = (rgb: number[]) => {
  const [r, g, b] = rgb.map((channel) => {
    const srgb = channel / 255;

    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4;
  });

  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};

/** The WCAG contrast ratio of two `#rrggbb` colours, from 1 to 21 (1 when either is unreadable). */
export const contrastRatio = (a: string, b: string) => {
  const [first, second] = [channels(a), channels(b)];

  if (!first || !second) return 1;

  const [light, dark] = [luminance(first), luminance(second)].sort((x, y) => y - x);

  return (light + 0.05) / (dark + 0.05);
};

/** `from` moved `percent` of the way to `to`, as `#rrggbb`. */
const mix = (from: number[], to: number[], percent: number) =>
  `#${from
    .map((channel, index) => Math.round(channel + ((to[index] - channel) * percent) / 100))
    .map((channel) => channel.toString(16).padStart(2, '0'))
    .join('')}`;

/**
 * What accent text needs against its background: WCAG's minimum for body text, since Sharkord draws
 * some of it (times beside a message) at 60% on top.
 */
const READABLE = 4.5;

/**
 * The accent as a colour for text and icons on `background`: the accent itself when it reads there,
 * otherwise moved towards `text` just until it does, so it keeps its hue as far as it can. An accent
 * picked for fills (a dark one on a dark theme) would leave times and icons invisible.
 */
export const readableAccent = (accent: string, background: string, text: string) => {
  const [from, to] = [channels(accent), channels(text)];

  if (!from || !to || contrastRatio(accent, background) >= READABLE) return accent;

  for (let percent = 10; percent < 100; percent += 10) {
    const mixed = mix(from, to, percent);

    if (contrastRatio(mixed, background) >= READABLE) return mixed;
  }

  return text;
};
