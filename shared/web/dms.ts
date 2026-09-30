/** The direct-message list both clients draw (`components/DmList.tsx`): its rows and its search. */

/** One conversation, as either client lists it: with someone, on one server, as one account. */
export type DmRow = {
  /** unique across every server's conversations (`dmKey`) */
  key: string;
  entryId: string;
  channelId: number;
  /** the other person */
  name: string;
  avatarUrl: string | null;
  serverName: string;
  /** which of the user's accounts on that server, when that needs saying */
  accountLabel: string | null;
  /** when the last message arrived, in milliseconds; what the list is ordered by */
  lastMessageAt: number | null;
};

export const dmKey = (entryId: string, channelId: number) => `${entryId}:${channelId}`;

/**
 * The rows whose person or server contains `query`, ignoring case: the two things a row shows, so a
 * search never matches on something that is not on screen.
 */
export const filterDms = <T extends Pick<DmRow, 'name' | 'serverName'>>(rows: readonly T[], query: string) => {
  const needle = query.trim().toLowerCase();

  if (!needle) return [...rows];

  return rows.filter((row) => row.name.toLowerCase().includes(needle) || row.serverName.toLowerCase().includes(needle));
};

/** A row's second line: the account, when there is one, then the server. */
export const dmMeta = ({ accountLabel, serverName }: Pick<DmRow, 'accountLabel' | 'serverName'>) =>
  accountLabel ? `${accountLabel} · ${serverName}` : serverName;

/** The letter on the avatar of someone without a picture. */
export const dmInitial = (name: string) => [...name.trim()][0]?.toUpperCase() ?? '?';
