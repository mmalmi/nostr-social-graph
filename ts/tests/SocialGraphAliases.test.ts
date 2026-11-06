import { describe, expect, it } from 'vitest';
import { SocialGraph } from '../src/SocialGraph';
import { UniqueIds } from '../src/UniqueIds';

const key = (n: number) => n.toString(16).padStart(2, '0').repeat(32);
type List = [number, number, number[], string?, boolean?];
function binary(version: number, ids: [number, string][], follows: List[], mutes: List[] = []) {
  const bytes: number[] = [];
  const put = (n: number) => { do { const b = n % 128; n = Math.floor(n / 128); bytes.push(b | (n ? 128 : 0)); } while (n); };
  const hex = (s: string) => { for (let i = 0; i < s.length; i += 2) bytes.push(parseInt(s.slice(i, i + 2), 16)); };
  put(version); put(ids.length);
  for (const [id, value] of ids) {
    if (version === 2) { hex(value); put(id); }
    else { put(id); put(0); hex(value); }
  }
  for (const lists of [follows, mutes]) {
    put(lists.length);
    for (const [owner, at, targets, revision, complete] of lists) {
      put(owner); put(at);
      if (version === 4) { bytes.push(revision ? (complete ? 2 : 1) : 0); if (revision) hex(revision); }
      put(targets.length); targets.forEach(put);
    }
  }
  return new Uint8Array(bytes);
}

describe('legacy node aliases', () => {
  for (const version of [2, 3, 4]) it(`preserves incoming follows and mutes from v${version}`, async () => {
    const raw = binary(version, [[0, key(10)], [1, key(11)], [2, key(12)], [7, key(12)]],
      [[0, 5, [7, 1]], [1, 6, [2, 7]]], [[0, 7, [2, 7]]]);
    const graph = await SocialGraph.fromBinary(key(10), raw);
    expect(graph.isFollowing(key(10), key(12))).toBe(true);
    expect(graph.getFollowDistance(key(12))).toBe(1);
    expect(graph.getFollowersByUser(key(12)).size).toBe(2);
    expect(graph.getMutedByUser(key(10)).size).toBe(1);
    expect([...graph.getInternalData().ids].length).toBe(3);
    const encoded = await graph.toBinary();
    expect(await (await SocialGraph.fromBinary(key(10), encoded)).toBinary()).toEqual(encoded);
  });

  it('selects a newer empty whole list over an older aliased list', async () => {
    const raw = binary(3, [[0, key(10)], [1, key(11)], [2, key(12)], [8, key(11)]],
      [[0, 5, [8]], [1, 10, [2]], [8, 11, []]], [[1, 10, [2]], [8, 11, []]]);
    const graph = await SocialGraph.fromBinary(key(10), raw);
    expect(graph.getFollowedByUser(key(11)).size).toBe(0);
    expect(graph.getMutedByUser(key(11)).size).toBe(0);
    expect(graph.getFollowersByUser(key(12)).size).toBe(0);
  });

  it('selects equal-time revisions by event ID, preserving complete metadata', async () => {
    const raw = binary(4, [[0, key(10)], [1, key(11)], [2, key(12)], [8, key(11)]],
      [[0, 5, [8]], [1, 10, [2], key(15), true], [8, 10, [], key(14), true]]);
    const graph = await SocialGraph.fromBinary(key(10), raw);
    expect(graph.getFollowedByUser(key(11)).size).toBe(0);
    expect(graph.getListRevision(key(11), 3)).toEqual({id: key(14), created_at: 10, complete: true});
    expect(graph.getFollowDistance(key(11))).toBe(1);
  });

  it('normalizes hex identity without changing arbitrary case-sensitive IDs', () => {
    const ids = new UniqueIds();
    expect(ids.id(key(10))).toBe(ids.id(key(10).toUpperCase()));
    expect(ids.has(key(10).toUpperCase())).toBe(true);
    expect(ids.id('Alice')).not.toBe(ids.id('alice'));
  });
});
