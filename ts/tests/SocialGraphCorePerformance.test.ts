import { describe, expect, it } from 'vitest';
import { SocialGraph } from '../src/SocialGraph';
import { fromBinary } from '../src/SocialGraphBinary';
import { SocialGraphUtils } from '../src/SocialGraphUtils';

const key = (id: number) => id.toString(16).padStart(64, '0');
const list = (author: number, targets: number[], created_at: number, kind: 3 | 10000 = 3, idNum?: number) => ({
  id: key(idNum ?? created_at * 1000 + author),
  sig: '',
  content: '',
  kind,
  pubkey: key(author),
  created_at,
  tags: targets.map((id) => ['p', key(id)]),
});

describe('core performance & correctness regressions', () => {
  it('isOvermuted handles closer followers, unreachable muters, root case, and float thresholds', () => {
    const graph = new SocialGraph(key(0));
    // 0 -> 1 -> 2, and 99 (unreachable) mutes 2
    graph.handleEvent([
      list(0, [1], 10),
      list(1, [2], 10),
      list(99, [2], 10, 10000),
    ], true);

    // Unreachable muter alone never overmutes
    expect(graph.isOvermuted(key(2), 1)).toBe(false);
    expect(graph.isOvermuted(key(2), Number.POSITIVE_INFINITY)).toBe(false);

    // Distance-2 muter beaten by distance-1 follower
    graph.handleEvent([
      list(1, [2, 3], 11),
      list(3, [2], 11, 10000),
    ], true);
    expect(graph.isOvermuted(key(2), 1)).toBe(false);
    expect(graph.isOvermuted(key(2), 100)).toBe(false);
    expect(graph.isOvermuted(key(2), Number.POSITIVE_INFINITY)).toBe(false);

    // Equal distance: 1 follower (1) and 1 muter (4) at distance 1
    graph.handleEvent([
      list(0, [1, 4], 12),
      list(4, [2], 12, 10000),
    ], true);
    expect(graph.isOvermuted(key(2), 0)).toBe(false);
    expect(graph.isOvermuted(key(2), -1)).toBe(false);
    expect(graph.isOvermuted(key(2), Number.NaN)).toBe(false);
    expect(graph.isOvermuted(key(2), 1)).toBe(false);
    expect(graph.isOvermuted(key(2), 1.01)).toBe(true);
    expect(graph.isOvermuted(key(2), Number.POSITIVE_INFINITY)).toBe(true);

    // Root is never overmuted regardless of hex case
    graph.handleEvent(list(1, [0], 13, 10000), true);
    expect(graph.isOvermuted(key(0).toUpperCase(), 100)).toBe(false);
  });

  it('handleEvent diff updates preserve exact distances on unchanged refreshes and simultaneous unfollow+follow', async () => {
    const graph = new SocialGraph(key(0));
    graph.handleEvent([
      list(0, [1, 2], 10),
      list(1, [10, 11, 12, 13], 10),
      list(10, [20], 10),
      list(11, [21], 10),
      list(2, [30], 10),
      list(30, [21], 10),
    ], true);

    // Unchanged follow list with newer timestamp updates revision without mutating edges
    expect(graph.handleEvent(list(1, [10, 11, 12, 13], 20, 3, 9001), true)).toBe(true);
    expect(graph.getListRevision(key(1), 3)).toEqual({ id: key(9001), created_at: 20, complete: true });
    expect(graph.getFollowDistance(key(20))).toBe(3);

    // Simultaneous bulk unfollow (10, 11) + new follow (14 -> 22)
    graph.handleEvent(list(14, [22], 15), true);
    graph.handleEvent(list(1, [12, 13, 14], 30), true);

    const liveDistances = [0, 1, 2, 10, 11, 12, 13, 14, 20, 21, 22, 30].map((u) => graph.getFollowDistance(key(u)));
    expect(liveDistances).toEqual([0, 1, 1, 1000, 1000, 2, 2, 2, 1000, 3, 3, 2]);

    await graph.recalculateFollowDistances(1000, 1e9, () => {});
    const recalcDistances = [0, 1, 2, 10, 11, 12, 13, 14, 20, 21, 22, 30].map((u) => graph.getFollowDistance(key(u)));
    expect(liveDistances).toEqual(recalcDistances);
  });

  it('batchRemoveUsers and removeMutedNotFollowedUsers clean all incident edges and empty distance buckets', async () => {
    const graph = new SocialGraph(key(0));
    graph.handleEvent([
      list(0, [1], 10),
      list(1, [2], 10),
      list(1, [3, 4], 10, 10000),
      list(3, [1], 10),
      list(3, [2], 10, 10000),
    ], true);

    const removed = await graph.removeMutedNotFollowedUsers();
    expect(removed).toBe(2); // 3 and 4 have muters and zero followers
    expect(graph.getMutedByUser(key(1)).size).toBe(0);
    expect(graph.getUserMutedBy(key(2)).size).toBe(0);
    expect(graph.getFollowersByUser(key(1))).toEqual(new Set([key(0)]));

    // Removing the only distance-2 user also removes the empty distance-2 bucket
    const id2 = graph.getInternalData().ids.get(key(2))!;
    SocialGraphUtils.batchRemoveUsers(graph, [id2]);
    expect(graph.size().sizeByDistance[2]).toBeUndefined();
    expect(graph.isFollowing(key(1), key(2))).toBe(false);
    expect(graph.getFollowersByUser(key(2)).size).toBe(0);
  });

  it('fromBinary preserves strict mobile checks and fast alias-free roundtrip with revisions', async () => {
    const graph = new SocialGraph(key(0));
    graph.handleEvent([
      list(0, [1, 2], 100, 3, 500),
      list(1, [3], 101, 3, 501),
      list(0, [4], 102, 10000, 502),
      list(2, [], 103, 3, 503),
    ], true);

    const bytes = await graph.toBinary();
    const restored = await fromBinary(key(0), bytes, true);
    expect(restored.size()).toEqual(graph.size());
    expect(restored.getListRevision(key(0), 3)).toEqual({ id: key(500), created_at: 100, complete: true });
    expect(restored.getListRevision(key(2), 3)).toEqual({ id: key(503), created_at: 103, complete: true });
    expect(restored.getFollowDistance(key(3))).toBe(2);
    expect(restored.isMuting(key(0), key(4))).toBe(true);
  });
});
