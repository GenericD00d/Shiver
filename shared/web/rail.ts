/** Rail helpers shared by both clients' rails and the mobile bridge's. */

type Placed = { position: number };

/** Up to two initials, for a server without a logo. */
export const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase() ?? '')
    .join('') || '?';

/** A sorted copy, by rail position. */
export const byPosition = <T extends Placed>(items: readonly T[]) => [...items].sort((a, b) => a.position - b.position);

/** A folder's servers, in their order within it. */
export const membersOf = <T extends Placed & { folderId: string | null }>(servers: readonly T[], folderId: string) =>
  byPosition(servers.filter((server) => server.folderId === folderId));

/**
 * Every server in the order the rail shows them: top-level servers and folders by position, each
 * folder's servers in their own order (collapsed or not).
 */
export const railOrder = <T extends Placed & { folderId: string | null }>(
  servers: readonly T[],
  folders: readonly (Placed & { id: string })[]
) =>
  byPosition([
    ...servers.filter((server) => !server.folderId).map((server) => ({ position: server.position, members: [server] })),
    ...folders.map((folder) => ({ position: folder.position, members: membersOf(servers, folder.id) }))
  ]).flatMap(({ members }) => members);
