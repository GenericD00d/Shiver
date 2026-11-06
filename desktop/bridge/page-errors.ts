/**
 * What the page throws, for Shiver's log file (`logs.rs`), as Sharkord's desktop app logs its pages'
 * errors: uncaught errors, unhandled rejections and `console.error`, each as one line. Tokens in
 * an address (signed file links carry one) are blanked first. Held until the core's next drain,
 * and only so many, so a page throwing in a loop cannot grow the queue or flood the file.
 */

/** Most errors held between drains; later ones are counted rather than kept. */
const MAX_HELD = 30;
const MAX_LENGTH = 1000;

/** `token=…` and its spellings in a query string, whatever the parameter's case. */
const TOKEN_PARAMETER = /([?&](?:access_?token|token|jwt|auth)=)[^&\s"'#]+/gi;
const BEARER = /(bearer\s+)[\w.-]+/gi;

let held: string[] = [];
let dropped = 0;

/** One value as a line of text: an error's name, message and the first of its stack. */
export function describe(value: unknown): string {
  if (value instanceof Error) {
    const at = value.stack?.split('\n').find((line) => line.trim().startsWith('at '))?.trim();

    return `${value.name}: ${value.message}${at ? ` (${at})` : ''}`;
  }

  if (typeof value === 'string') return value;

  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

/** The line as it may be written: tokens blanked, one line, bounded. */
export function redact(line: string): string {
  return line
    .replace(TOKEN_PARAMETER, '$1…')
    .replace(BEARER, '$1…')
    .replace(/\s*\n\s*/g, ' ⏎ ')
    .slice(0, MAX_LENGTH);
}

export function note(...values: unknown[]) {
  const line = redact(values.map(describe).join(' '));

  if (!line) return;

  if (held.length >= MAX_HELD) {
    dropped += 1;

    return;
  }

  held.push(line);
}

/** The errors since the last drain, and how many more there were. */
export function takePageErrors(): string[] {
  const taken = held;

  if (dropped > 0) taken.push(`…and ${dropped} more`);

  held = [];
  dropped = 0;

  return taken;
}

/** Starts listening; at document start, so errors while the client loads are caught too. */
export function capturePageErrors() {
  window.addEventListener('error', (event) => note(event.error ?? event.message));
  window.addEventListener('unhandledrejection', (event) => note('unhandled rejection:', event.reason));

  const original = console.error.bind(console);

  console.error = (...values: unknown[]) => {
    note(...values);
    original(...values);
  };
}
