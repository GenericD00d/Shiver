/** The muted-channel list and shared unread floor, validated and written through `rows`. */

export const MAX_MUTED_CHANNELS = 500;
export const MAX_FLOOR_CHANNELS = 5000;

/** Positive integer channel ids, de-duplicated and capped. */
export const mutedFrom = (value) =>
  Array.isArray(value)
    ? [...new Set(value.filter((id) => Number.isInteger(id) && id > 0))].slice(0, MAX_MUTED_CHANNELS)
    : [];

/** `{ "<channel id>": count }` with valid keys and non-negative counts (rounded), or null. */
export const floorFrom = (value) => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;

  const floor = {};
  let kept = 0;

  for (const [key, count] of Object.entries(value)) {
    if (kept >= MAX_FLOOR_CHANNELS) break;
    if (!/^[1-9]\d{0,15}$/.test(key) || typeof count !== 'number' || !Number.isFinite(count) || count < 0) {
      continue;
    }

    floor[key] = Math.min(Math.round(count), 0xffffffff);
    kept += 1;
  }

  return floor;
};

export const createSettings = (rows, onChange = () => undefined) => ({
  getMutedChannels: async (userId) => ({ mutedChannels: mutedFrom((await rows.readRow(userId)).mutedChannels) }),

  setMutedChannels: async (userId, value) => {
    const mutedChannels = mutedFrom(value);

    onChange(userId, await rows.updateRow(userId, (row) => ({ ...row, mutedChannels })));

    return { mutedChannels };
  },

  setReadFloor: async (userId, value) => {
    const readFloor = floorFrom(value);

    if (!readFloor) throw new Error('That is not an unread floor');

    await rows.updateRow(userId, (row) => ({ ...row, readFloor }));

    return { ok: true };
  }
});
