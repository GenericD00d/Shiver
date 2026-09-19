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
 * download its plain filename — `photo~4f3a91b2c0d8e1f9.png` rather than `photo.png` — a small price
 * for never serving the wrong picture, and is what the host would have to do to fix this properly.
 *
 * Not a deletion, deliberately: the plugin SDK has no file API, so a plugin cannot remove anything
 * from the store. It does not need to. Sharkord's own `cleanupFiles` cron already deletes orphaned
 * files and their rows every fifteen minutes; the bug was never that they lingered.
 */

import { randomBytes } from 'node:crypto';

/**
 * Enough that two uploads of the same name will not collide.
 *
 * It was 3 bytes. Twenty-four bits sounds like plenty and is not: the birthday bound puts a
 * collision at even odds after roughly 4,800 uploads sharing one base name, and a collision here is
 * exactly the stale-picture bug this file exists to prevent. Eight bytes costs ten more characters
 * in a filename and takes the number out of reach.
 */
const SUFFIX_BYTES = 8;

/** A file's name split at the last dot, so the suffix lands before the extension. */
export const splitName = (name) => {
  const at = name.lastIndexOf('.');

  return at > 0 ? { base: name.slice(0, at), ext: name.slice(at) } : { base: name, ext: '' };
};

/**
 * The marker that says a suffix is ours.
 *
 * **Not a pattern matched against the uploader's name.** It was: anything ending `-` plus six hex
 * digits was taken as already suffixed and passed through untouched — so calling a file
 * `photo-4f3a91.png` opted it out of the randomisation entirely and stored it at exactly the
 * predictable address this module exists to eliminate. The one thing a guard against
 * double-suffixing must not be is something the attacker can write.
 *
 * `installFileNaming` instead renames each file once per save and never looks at the result again,
 * so there is no second pass to guard against. `uniqueName` stays pure and always appends.
 */
const SUFFIX_MARK = '~';

export const uniqueName = (name, random = randomBytes(SUFFIX_BYTES).toString('hex')) => {
  const { base, ext } = splitName(name);

  return `${base}${SUFFIX_MARK}${random}${ext}`;
};

export const installFileNaming = (ctx) => {
  ctx.hooks.onBeforeFileSave(async ({ originalName, type }) => {
    // Everything a server stores by name is worth this, not only attachments: a new avatar at the
    // old avatar's address is the same stale picture, shown against somebody's name.
    //
    // Always renamed, unconditionally. The hook runs once per save and the name it returns is not
    // fed back through here, so there is nothing to detect and nothing an uploader can imitate to
    // skip it.
    const renamed = uniqueName(String(originalName ?? 'file'));

    ctx.logger.debug(`Shiver: storing a ${type} as ${renamed}`);

    return { update: { originalName: renamed } };
  });
};
