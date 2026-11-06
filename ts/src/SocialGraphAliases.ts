import { canonicalNodeId } from './UniqueIds';

type List = [number, number[], number];
type Revision = [number, number, string, number, boolean];

/** Repair legacy aliases without combining obsolete replaceable lists. */
export function canonicalizeGraphNodes(
  uniqueIds: [string, number][], followLists: List[], muteLists: List[], revisions: Revision[],
) {
  const canonical = new Map<string, number>();
  const byId = new Map<number, string>();
  for (const [raw, id] of uniqueIds) {
    const value = canonicalNodeId(raw);
    if (byId.has(id) && byId.get(id) !== value) throw new Error('Conflicting graph node ID');
    byId.set(id, value);
    canonical.set(value, Math.min(canonical.get(value) ?? id, id));
  }
  const aliases = new Map<number, number>();
  for (const [id, value] of byId) {
    if (canonical.get(value) !== id) aliases.set(id, canonical.get(value)!);
  }
  const ids: [string, number][] = [...canonical];
  if (!aliases.size) return { uniqueIds: ids, followLists, muteLists, revisions };
  const map = (id: number) => aliases.get(id) ?? id;
  const metadata = new Map(revisions.map(r => [`${r[1]}:${r[0]}`, r]));
  const selectedRevisions: Revision[] = [];
  function lists(input: List[], kind: number): List[] {
    const chosen = new Map<number, { list: List; revision?: Revision; conflict?: boolean }>();
    for (const list of input) {
      const [owner, , at] = list;
      const candidate = metadata.get(`${kind}:${owner}`);
      const revision = candidate?.[3] === at && /^[0-9a-f]{64}$/.test(candidate[2]) ? candidate : undefined;
      const key = map(owner);
      const old = chosen.get(key);
      let conflict = false;
      if (old && at === old.list[2]) {
        const resolved = revision && old.revision &&
          (revision[2] !== old.revision[2] || revision[4] !== old.revision[4]);
        if (!resolved) {
          const a = new Set(list[1].map(map));
          const b = new Set(old.list[1].map(map));
          if (a.size !== b.size || [...a].some(id => !b.has(id))) {
            if (kind === 3) old.conflict = true;
            else {
              old.list[1] = [...new Set([...b, ...a])];
              old.revision = undefined;
              continue;
            }
          }
        }
        conflict = !!old.conflict;
      }
      let replace = !old;
      if (old) {
        if (at !== old.list[2]) replace = at > old.list[2];
        else if (revision && old.revision && revision[2] !== old.revision[2]) replace = revision[2] < old.revision[2];
        else if (revision && old.revision && revision[4] !== old.revision[4]) replace = revision[4];
        else if (!!revision !== !!old.revision) replace = !!revision;
        else replace = owner < old.list[0];
      }
      if (replace) chosen.set(key, { list, revision, conflict });
    }
    return [...chosen].map(([owner, { list: [, targets, at], revision, conflict }]) => {
      if (conflict) throw new Error('Snapshot contains conflicting latest follow lists');
      if (revision) selectedRevisions.push([owner, kind, revision[2], at, revision[4]]);
      return [owner, [...new Set(targets.map(map))], at];
    });
  }
  const follows = lists(followLists, 3);
  const mutes = lists(muteLists, 10000);
  return { uniqueIds: ids, followLists: follows, muteLists: mutes, revisions: selectedRevisions };
}
