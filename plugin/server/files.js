/**
 * Gives every stored file an unpredictable name.
 *
 * Sharkord reuses a filename once the old file is deleted, and clients cache `/public/<name>` for an
 * hour, so a new upload could show up as someone's old, deleted picture. A random suffix prevents it.
 */

import { Buffer } from 'node:buffer';
import { randomBytes } from 'node:crypto';

/** 64 bits: a birthday collision on one base name is out of reach. */
const SUFFIX_BYTES = 8;
const SUFFIX_MARK = '~';
/** Keeps name + suffix + extension under the usual 255-byte filesystem limit. */
const MAX_BASE_BYTES = 180;

/** Splits at the last dot, so the suffix lands before the extension. */
export const splitName = (name) => {
  const at = name.lastIndexOf('.');

  return at > 0 ? { base: name.slice(0, at), ext: name.slice(at) } : { base: name, ext: '' };
};

/** Cuts text to at most `limit` UTF-8 bytes without splitting a character. */
const clampBytes = (text, limit) => {
  let out = '';
  let bytes = 0;

  for (const character of text) {
    bytes += Buffer.byteLength(character);

    if (bytes > limit) break;

    out += character;
  }

  return out;
};

/**
 * `photo.png` -> `photo~<random>.png`. Always appends; the hook never sees its own output. Path
 * separators and control characters become `_`, whatever the host does with the name afterwards.
 */
export const uniqueName = (raw, random = randomBytes(SUFFIX_BYTES).toString('hex')) => {
  const name = raw.replace(/[/\\\x00-\x1f\x7f]/g, '_');
  const { base, ext } = splitName(name);
  const [stem, tail] = Buffer.byteLength(ext) > 32 ? [name, ''] : [base, ext];

  return `${clampBytes(stem, MAX_BASE_BYTES)}${SUFFIX_MARK}${random}${tail}`;
};

/** Installs the rename hook and returns the host's unsubscribe, for unload. */
export const installFileNaming = (ctx) =>
  ctx.hooks.onBeforeFileSave(async ({ originalName, type }) => {
    const renamed = uniqueName(String(originalName ?? 'file'));

    ctx.logger.debug(`Shiver: storing a ${type} as ${renamed}`);

    return { update: { originalName: renamed } };
  });
