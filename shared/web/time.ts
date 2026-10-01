/** Times as Shiver's own pages say them. */

/** How long ago `at` (milliseconds) was: "just now", "5m ago", "3h ago", "2d ago". */
export const relativeTime = (at: number, now = Date.now()) => {
  const seconds = Math.max(0, Math.round((now - at) / 1000));

  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;

  return `${Math.floor(seconds / 86400)}d ago`;
};
