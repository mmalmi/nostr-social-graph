import {describe, expect, it} from 'vitest';
import {SocialGraph} from '../src/SocialGraph';

const key = (id: number) => id.toString(16).padStart(64, '0');
const list = (author: number, follows: number[], created_at: number, kind = 3) => ({
  id: key(created_at * 1000 + author), sig: '', content: '', kind, pubkey: key(author), created_at,
  tags: follows.map(id => ['p', key(id)]),
});

/** Distances and buckets as a from-scratch breadth-first search sees them. */
async function expectExact(graph: SocialGraph, users: number) {
  const distances = Array.from({length: users}, (_, id) => graph.getFollowDistance(key(id)));
  const buckets = new Map<number, Set<string>>();
  for (let d = 0; d < users; d++) {
    const bucket = graph.getUsersByFollowDistance(d);
    if (bucket.size) buckets.set(d, bucket);
  }
  const size = graph.size();
  await graph.recalculateFollowDistances(1000, 1e9, () => {});
  expect(distances).toEqual(Array.from({length: users}, (_, id) => graph.getFollowDistance(key(id))));
  for (let d = 0; d < users; d++) {
    expect(buckets.get(d) ?? new Set()).toEqual(graph.getUsersByFollowDistance(d));
  }
  expect(size).toEqual(graph.size());
}

describe('follow distances without a recalculation', () => {
  it('reaches the follows of a newly followed user whose list was already known', () => {
    const graph = new SocialGraph(key(0));
    graph.handleEvent([list(1, [2, 3], 10), list(2, [4], 10)], true);
    expect(graph.getFollowDistance(key(4))).toBe(1000);

    graph.handleEvent(list(0, [1], 10));

    expect([1, 2, 3, 4].map(id => graph.getFollowDistance(key(id)))).toEqual([1, 2, 2, 3]);
    expect(graph.getUsersByFollowDistance(2)).toEqual(new Set([key(2), key(3)]));
  });

  it('moves an unfollowed user and everyone reached through them', () => {
    const graph = new SocialGraph(key(0));
    graph.handleEvent([list(0, [1, 5], 10), list(1, [2], 10), list(2, [3], 10), list(5, [6], 10), list(6, [7], 10), list(7, [3], 10)], true);
    expect(graph.getFollowDistance(key(3))).toBe(3);

    graph.handleEvent(list(0, [5], 20));

    expect([1, 2, 3].map(id => graph.getFollowDistance(key(id)))).toEqual([1000, 1000, 4]);
    expect(graph.getUsersByFollowDistance(1)).toEqual(new Set([key(5)]));
    expect(graph.getUsersByFollowDistance(4)).toEqual(new Set([key(3)]));
    expect(graph.size().users).toBe(5);
  });

  it('keeps a distance that another equally short path still supports', () => {
    const graph = new SocialGraph(key(0));
    graph.handleEvent([list(0, [1, 2], 10), list(1, [3], 10), list(2, [3], 10), list(3, [4], 10)], true);
    graph.removeFollower(key(1), key(3));
    expect([3, 4].map(id => graph.getFollowDistance(key(id)))).toEqual([2, 3]);
  });

  it('matches a full recalculation after random follows and unfollows', async () => {
    let seed = 7;
    const random = (n: number) => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) % n;
    for (const [users, steps] of [[8, 400], [40, 1500], [150, 2500]]) {
      const graph = new SocialGraph(key(0));
      for (let step = 1; step <= steps; step++) {
        const a = random(users), b = random(users);
        const action = random(10);
        if (action < 5) graph.addFollower(key(a), key(b));
        else if (action < 8) graph.removeFollower(key(a), key(b));
        else graph.handleEvent(list(a, Array.from({length: random(6)}, () => random(users)), step), true);
        if (step % 50 === 0) await expectExact(graph, users);
      }
      await expectExact(graph, users);
    }
  });
});

describe('queries about unknown users', () => {
  it('answer without remembering the queried keys', () => {
    const graph = new SocialGraph(key(0));
    graph.handleEvent(list(0, [1], 10));
    const stranger = key(99), ids = graph.getInternalData().ids;
    const known = ids.size;

    expect(graph.getFollowDistance(stranger)).toBe(1000);
    expect(graph.isFollowing(stranger, key(1))).toBe(false);
    expect(graph.isFollowing(key(0), stranger)).toBe(false);
    expect(graph.followerCount(stranger)).toBe(0);
    expect(graph.followedByFriendsCount(stranger)).toBe(0);
    expect(graph.mutedByFriendsCount(stranger)).toBe(0);
    expect(graph.followedByFriends(stranger).size).toBe(0);
    expect(graph.getFollowedByUser(stranger).size).toBe(0);
    expect(graph.getFollowedByUser(stranger, true)).toEqual(new Set([stranger]));
    expect(graph.getFollowersByUser(stranger).size).toBe(0);
    expect(graph.getMutedByUser(stranger).size).toBe(0);
    expect(graph.getUserMutedBy(stranger).size).toBe(0);
    expect(graph.getFollowListCreatedAt(stranger)).toBeUndefined();
    expect(graph.getMuteListCreatedAt(stranger)).toBeUndefined();
    expect(graph.stats(stranger)).toEqual({});
    // An ignored list from an unknown author is not remembered either.
    expect(graph.handleEvent(list(99, [1], 10))).toBe(false);

    expect(ids.size).toBe(known);
    expect(ids.has(stranger)).toBe(false);
  });

  it('answer mute questions without building sets', () => {
    const graph = new SocialGraph(key(0));
    graph.handleEvent([list(0, [1, 2], 10), list(1, [3], 10), list(2, [], 10)], true);
    graph.handleEvent([list(1, [4], 11, 10000), list(2, [3, 4], 11, 10000)], true);
    expect(graph.isMuting(key(1), key(4))).toBe(true);
    expect(graph.isMuting(key(1), key(3))).toBe(false);
    expect(graph.isMuting(key(98), key(99))).toBe(false);
    // One friend follows 3 and one mutes: not outweighed. Nobody follows 4.
    expect(graph.isOvermuted(key(3))).toBe(false);
    expect(graph.isOvermuted(key(3), 2)).toBe(true);
    expect(graph.isOvermuted(key(4))).toBe(true);
    expect(graph.isOvermuted(key(99))).toBe(false);
  });

  it('still match keys regardless of hex case', () => {
    const graph = new SocialGraph(key(0xa));
    graph.handleEvent(list(0xa, [0xb], 10));
    expect(graph.getFollowDistance(key(0xb).toUpperCase())).toBe(1);
    expect(graph.isFollowing(key(0xa).toUpperCase(), key(0xb).toUpperCase())).toBe(true);
  });
});
