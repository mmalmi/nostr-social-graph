import { describe, expect, it } from 'vitest';
import { SocialGraph } from '../src/SocialGraph';

const key = (id: number) => id.toString(16).padStart(64, '0');
const root = key(1);
const author = key(2);
const oldTarget = key(3);
const newTarget = key(4);
const decoy = key(5);

function list(graph: SocialGraph, kind: 3 | 10000, pubkey: string, targets: string[], created_at: number) {
  graph.handleEvent({
    id: '', sig: '', content: '', kind, pubkey, created_at,
    tags: targets.map(target => ['p', target]),
  }, true, Infinity);
}

function snapshot(graph: SocialGraph) {
  const { ids, str: _str, ...maps } = graph.getInternalData();
  return {
    ids: Array.from(ids),
    maps: Object.fromEntries(Object.entries(maps).map(([name, map]) => [
      name, Array.from(map as Map<number, number | Set<number>>, ([id, value]) =>
        [id, value instanceof Set ? Array.from(value) : value]),
    ])),
    distances: Array.from(ids, ([, pubkey]) => [pubkey, graph.getFollowDistance(pubkey)]),
  };
}

describe('SocialGraph.merge', () => {
  it('keeps a retained root mute attached to its pubkey when bundled IDs differ', async () => {
    const retained = new SocialGraph(root);
    list(retained, 10000, root, [oldTarget], 10);
    const bundled = new SocialGraph(root);
    bundled.addFollower(root, decoy);

    await bundled.merge(retained);

    expect(bundled.getMutedByUser(root)).toEqual(new Set([oldTarget]));
    expect(bundled.getUserMutedBy(oldTarget)).toEqual(new Set([root]));
    expect(bundled.getUserMutedBy(decoy)).toEqual(new Set());
    // A source without a follow list must not erase a bundled follow list.
    expect(bundled.getFollowedByUser(root)).toEqual(new Set([decoy]));
    expect(bundled.getFollowDistance(decoy)).toBe(1);
  });

  it.each([3, 10000] as const)('remaps both author and target IDs for kind %i and updates reverse indexes', async kind => {
    const source = new SocialGraph(root);
    list(source, 3, root, [author], 10);
    list(source, kind, author, [newTarget], 30);
    const destination = new SocialGraph(root);
    destination.getFollowDistance(decoy);
    destination.getFollowDistance(oldTarget);
    destination.getFollowDistance(newTarget);
    list(destination, 3, root, [author], 10);
    list(destination, kind, author, [oldTarget], 20);
    await source.recalculateFollowDistances();
    const before = snapshot(source);
    expect(source.getInternalData().ids.id(author)).not.toBe(destination.getInternalData().ids.id(author));
    expect(source.getInternalData().ids.id(newTarget)).not.toBe(destination.getInternalData().ids.id(newTarget));

    await destination.merge(source);

    const forward = kind === 3 ? 'getFollowedByUser' : 'getMutedByUser';
    const reverse = kind === 3 ? 'getFollowersByUser' : 'getUserMutedBy';
    const timestamp = kind === 3 ? 'getFollowListCreatedAt' : 'getMuteListCreatedAt';
    expect(destination[forward](author)).toEqual(new Set([newTarget]));
    expect(destination[forward](decoy)).toEqual(new Set());
    expect(destination[reverse](newTarget)).toEqual(new Set([author]));
    expect(destination[reverse](oldTarget)).toEqual(new Set());
    expect(destination[timestamp](author)).toBe(30);
    expect(destination.getFollowDistance(root)).toBe(0);
    expect(destination.getFollowDistance(author)).toBe(1);
    expect(destination.getFollowDistance(newTarget)).toBe(kind === 3 ? 2 : 1000);
    expect(destination.getFollowDistance(oldTarget)).toBe(1000);
    expect(snapshot(source)).toEqual(before);
  });

  it.each([3, 10000] as const)('applies a newer empty kind %i list even when its author is disconnected in the source', async kind => {
    const destination = new SocialGraph(root);
    list(destination, 3, root, [author], 10);
    list(destination, kind, author, [oldTarget], 20);
    const source = new SocialGraph(decoy);
    source.getFollowDistance(newTarget);
    list(source, kind, author, [], 30);
    await source.recalculateFollowDistances();
    expect(source.getFollowDistance(author)).toBe(1000);

    await destination.merge(source);

    expect(destination.getRoot()).toBe(root);
    const forward = kind === 3 ? 'getFollowedByUser' : 'getMutedByUser';
    const reverse = kind === 3 ? 'getFollowersByUser' : 'getUserMutedBy';
    const timestamp = kind === 3 ? 'getFollowListCreatedAt' : 'getMuteListCreatedAt';
    expect(destination[forward](author)).toEqual(new Set());
    expect(destination[reverse](oldTarget)).toEqual(new Set());
    expect(destination[timestamp](author)).toBe(30);
    expect(destination.getFollowDistance(author)).toBe(1);
    expect(destination.getFollowDistance(oldTarget)).toBe(1000);
    // Replaying the older list cannot restore the removed relationship.
    list(destination, kind, author, [oldTarget], 20);
    expect(destination[forward](author)).toEqual(new Set());
  });

  it.each([3, 10000] as const)('preserves newer and equal destination kind %i lists, including timestamp zero', async kind => {
    for (const [ours, theirs] of [[30, 20], [20, 20], [0, 0]]) {
      const destination = new SocialGraph(root);
      destination.getFollowDistance(oldTarget);
      destination.getFollowDistance(newTarget);
      list(destination, kind, root, [oldTarget], ours);
      const source = new SocialGraph(root);
      source.getFollowDistance(oldTarget);
      source.getFollowDistance(newTarget);
      list(source, kind, root, [newTarget], theirs);

      await destination.merge(source);

      const forward = kind === 3 ? 'getFollowedByUser' : 'getMutedByUser';
      const reverse = kind === 3 ? 'getFollowersByUser' : 'getUserMutedBy';
      const timestamp = kind === 3 ? 'getFollowListCreatedAt' : 'getMuteListCreatedAt';
      expect(destination[forward](root)).toEqual(new Set([oldTarget]));
      expect(destination[reverse](oldTarget)).toEqual(new Set([root]));
      expect(destination[reverse](newTarget)).toEqual(new Set());
      expect(destination[timestamp](root)).toBe(ours);
    }
  });

  it('merges timestamp-free programmatic follows without aliasing the source sets', async () => {
    const destination = new SocialGraph(root);
    destination.getFollowDistance(decoy);
    const source = new SocialGraph(root);
    source.addFollower(root, author);
    source.addFollower(author, newTarget);
    const before = snapshot(source);

    await destination.merge(source);

    expect(destination.getFollowedByUser(root)).toEqual(new Set([author]));
    expect(destination.getFollowersByUser(newTarget)).toEqual(new Set([author]));
    expect(destination.getFollowDistance(newTarget)).toBe(2);
    expect(destination.getFollowListCreatedAt(root)).toBeUndefined();
    destination.removeFollower(author, newTarget);
    expect(snapshot(source)).toEqual(before);
  });

  it('recomputes all distances after a newer root list removes a whole branch', async () => {
    const destination = new SocialGraph(root);
    list(destination, 3, root, [author], 10);
    list(destination, 3, author, [oldTarget], 10);
    const source = new SocialGraph(root);
    list(source, 3, root, [], 20);

    await destination.merge(source);

    expect(destination.getFollowedByUser(root)).toEqual(new Set());
    expect(destination.getFollowersByUser(author)).toEqual(new Set());
    expect(destination.getFollowDistance(root)).toBe(0);
    expect(destination.getFollowDistance(author)).toBe(1000);
    expect(destination.getFollowDistance(oldTarget)).toBe(1000);
    expect(destination.getUsersByFollowDistance(1)).toEqual(new Set());
    expect(destination.getUsersByFollowDistance(2)).toEqual(new Set());
  });
});
