/**
 * Making sure one upload's address is never handed to another.
 *
 * Sharkord names a stored file after what the uploader called it, and makes it unique only against
 * the files it currently has: `getUniqueName` looks for a row with that name and, finding none,
 * takes the name. Deleting a message frees its row, and the orphan cron then deletes the file — so
 * `photo.png` becomes available again, and the next `photo.png` is stored at exactly the URL the old
 * one had.
 *
 * That URL is served `public, max-age=3600, must-revalidate`, so every client that saw the old file
 * keeps showing it for an hour. The symptom is a new image arriving as somebody's old, deleted one,
 * which is what this exists to stop.
 *
 * The fix is only to make the name unpredictable: a short random suffix, added before the file is
 * stored, so two uploads can never collide however many messages are deleted in between. It costs a
 * download its plain filename — `photo-4f3a91.png` rather than `photo.png` — which is a small price
 * for never serving the wrong picture, and is what the host would have to do to fix this properly.
 *
 * Not a deletion, deliberately: the plugin SDK has no file API, so a plugin cannot remove anything
 * from the store. It does not need to. Sharkord's own `cleanupFiles` cron already deletes orphaned
 * files and their rows every fifteen minutes; the bug was never that they lingered.
 */

import { randomBytes } from 'node:crypto';

/** Enough to make a collision impossible in practice, short enough not to disfigure a filename. */
const SUFFIX_BYTES = 3;

/** A file's name split at the last dot, so the suffix lands before the extension. */
export const splitName = (name) => {
  const at = name.lastIndexOf('.');

  return at > 0 ? { base: name.slice(0, at), ext: name.slice(at) } : { base: name, ext: '' };
};

/** Already carrying one of ours, which happens when a hook runs over a file twice. */
const SUFFIXED = /-[0-9a-f]{6}$/;

export const uniqueName = (name, random = randomBytes(SUFFIX_BYTES).toString('hex')) => {
  const { base, ext } = splitName(name);

  if (SUFFIXED.test(base)) return name;

  return `${base}-${random}${ext}`;
};

export const installFileNaming = (ctx) => {
  ctx.hooks.onBeforeFileSave(async ({ originalName, type }) => {
    // Everything a server stores by name is worth this, not only attachments: a new avatar at the
    // old avatar's address is the same stale picture, shown against somebody's name.
    const renamed = uniqueName(String(originalName ?? 'file'));

    if (renamed === originalName) return;

    ctx.logger.debug(`Shiver: storing a ${type} as ${renamed}`);

    return { update: { originalName: renamed } };
  });
};
