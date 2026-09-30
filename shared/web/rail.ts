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

/** One row of the rail's top level (`rail::RailRef`). */
export type RailRef = { kind: 'server' | 'folder'; id: string };

/** Where a drop lands on a tile: before or after it, or `into` it (a folder, or a new one with it). */
export type DropZone = 'before' | 'after' | 'into';

/** One call a drop makes, in order: the clients map each to their own command. */
export type RailStep =
  | { op: 'setFolder'; serverId: string; folderId: string | null }
  | { op: 'createFolder'; memberIds: string[] }
  | { op: 'reorder'; ordered: RailRef[] }
  | { op: 'reorderInFolder'; ids: string[] };

type Member = Placed & { id: string; folderId: string | null };

const place = <T>(list: T[], item: T, at: number, zone: DropZone) => {
  list.splice(zone === 'after' ? at + 1 : at, 0, item);

  return list;
};

/**
 * What dropping `source` on `target` does, as the calls to make. `into` a folder moves a server in;
 * onto a loose server it makes a folder of the two. Before or after a tile places the source there,
 * joining or leaving the folder that tile is in. Folders do not nest, and a folder left with fewer
 * than two servers dissolves in the core (`Rail::prune_folders`), so the order sent afterwards
 * names what will be there then.
 */
export const railDrop = (
  servers: readonly Member[],
  folders: readonly (Placed & { id: string })[],
  source: RailRef,
  target: RailRef,
  zone: DropZone
): RailStep[] => {
  if (source.kind === target.kind && source.id === target.id) return [];

  const moving = source.kind === 'server' ? servers.find((server) => server.id === source.id) : undefined;
  const aimed = target.kind === 'server' ? servers.find((server) => server.id === target.id) : undefined;

  if (source.kind === 'server' && !moving) return [];
  if (target.kind === 'server' && !aimed) return [];
  if (target.kind === 'folder' && !folders.some((folder) => folder.id === target.id)) return [];

  if (zone === 'into') {
    if (!moving) return [];

    if (target.kind === 'folder') {
      return moving.folderId === target.id ? [] : [{ op: 'setFolder', serverId: moving.id, folderId: target.id }];
    }

    // a server inside a folder is not a place to make another: folders do not nest
    return aimed && !aimed.folderId ? [{ op: 'createFolder', memberIds: [aimed.id, moving.id] }] : [];
  }

  const scope = aimed?.folderId ?? null;

  // a folder goes only among the top level
  if (source.kind === 'folder' && scope) return [];

  const steps: RailStep[] = [];

  if (moving && moving.folderId !== scope) {
    steps.push({ op: 'setFolder', serverId: moving.id, folderId: scope });
  }

  if (scope) {
    const ids = membersOf(servers, scope)
      .map((server) => server.id)
      .filter((id) => id !== source.id);

    steps.push({ op: 'reorderInFolder', ids: place(ids, source.id, ids.indexOf(target.id), zone) });

    return steps;
  }

  // the folder the server leaves, if that leaves it too few to stay one
  const left = moving?.folderId ?? null;
  const remaining = left ? membersOf(servers, left).filter((server) => server.id !== source.id) : [];
  const dissolved = left && remaining.length < 2 ? left : null;

  const rows: { position: number; refs: RailRef[] }[] = [
    ...servers
      .filter((server) => !server.folderId)
      .map((server) => ({ position: server.position, refs: [{ kind: 'server' as const, id: server.id }] })),
    ...folders.map((folder) => ({
      position: folder.position,
      refs:
        folder.id === dissolved
          ? remaining.map((server): RailRef => ({ kind: 'server', id: server.id }))
          : [{ kind: 'folder' as const, id: folder.id }]
    }))
  ];
  const top = byPosition(rows)
    .flatMap(({ refs }) => refs)
    .filter((ref) => !(ref.kind === source.kind && ref.id === source.id));

  const at = top.findIndex((ref) => ref.kind === target.kind && ref.id === target.id);

  if (at < 0) return steps;

  steps.push({ op: 'reorder', ordered: place(top, source, at, zone) });

  return steps;
};
