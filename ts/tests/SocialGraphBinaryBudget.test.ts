import {describe, expect, it} from 'vitest';
import {SocialGraph} from '../src/SocialGraph';

class CountedSet extends Set<number> {
  visited = 0;
  override *[Symbol.iterator]() {
    for (const value of super[Symbol.iterator]()) {
      this.visited++;
      yield value;
    }
  }
}

describe('binary export budgets', () => {
  it.each([
    {name: 'edge', limits: [undefined, 1]},
    {name: 'node', limits: [2]},
    {name: 'per-owner edge', limits: [undefined, undefined, undefined, 1]},
  ])('stops reading edges once the $name budget is exhausted', async ({limits}) => {
    const graph = new SocialGraph('root');
    for (let i = 0; i < 5_000; i++) graph.addFollower('root', `user:${i}`);
    const data = graph.getInternalData();
    const rootId = data.ids.id('root');
    const targets = new CountedSet(data.followedByUser.get(rootId));
    data.followedByUser.set(rootId, targets);

    const binary = await graph.toBinary(...limits);

    // Both planning and writing may inspect the next edge before stopping.
    expect(targets.visited).toBeLessThanOrEqual(4);
    const restored = await SocialGraph.fromBinary('root', binary);
    expect([...restored.getFollowedByUser('root')]).toEqual(['user:0']);
  });

  it.each([
    {limits: [undefined, 3], hex: '0304000204726f6f7401020566697273740202067365636f6e640402056d7574656401007b0201020100c8030104'},
    {limits: [undefined, undefined, 0], hex: '0304000204726f6f7401020566697273740202067365636f6e640402056d7574656401007b0201020100c8030104'},
    {limits: [undefined, undefined, undefined, 1], hex: '0303000204726f6f740102056669727374030205746869726402007b01010100010300'},
  ])('preserves existing binary order and timestamps with limits $limits', async ({limits, hex}) => {
    const graph = mixedGraph();
    expect(await graph.toBinary(...limits)).toEqual(new Uint8Array(Buffer.from(hex, 'hex')));
  });

  it('preserves zero-as-unlimited edge and node budgets', async () => {
    const binary = await mixedGraph().toBinary(0, 0, undefined, 0);
    const restored = await SocialGraph.fromBinary('root', binary);
    expect([...restored.getFollowedByUser('root')]).toEqual(['first', 'second']);
    expect(restored.isFollowing('first', 'third')).toBe(true);
    expect([...restored.getMutedByUser('root')]).toEqual(['muted']);
  });
});

function mixedGraph() {
  const graph = new SocialGraph('root');
  graph.addFollower('root', 'first');
  graph.addFollower('root', 'second');
  graph.addFollower('first', 'third');
  const data = graph.getInternalData();
  const rootId = data.ids.id('root');
  data.mutedByUser.set(rootId, new Set([data.ids.id('muted')]));
  data.followListCreatedAt.set(rootId, 123);
  data.muteListCreatedAt.set(rootId, 456);
  return graph;
}
