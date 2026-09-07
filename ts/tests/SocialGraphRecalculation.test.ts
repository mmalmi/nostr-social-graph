import {afterEach, describe, expect, it, vi} from 'vitest';
import {SocialGraph} from '../src/SocialGraph';

const key = (id: number) => id.toString(16).padStart(64, '0');
const chain = (length: number) => {
  const graph = new SocialGraph(key(0));
  for (let i = 1; i < length; i++) graph.addFollower(key(i - 1), key(i));
  return graph;
};

afterEach(() => vi.useRealTimers());

describe('batched follow-distance recalculation', () => {
  it('uses the batch budget across newly discovered users', async () => {
    vi.useFakeTimers();
    const graph = chain(100);
    const pending = graph.recalculateFollowDistances(100, 1000, () => {});
    await Promise.resolve();
    expect(vi.getTimerCount()).toBe(0);
    await pending;
    expect(graph.getFollowDistance(key(99))).toBe(99);
  });

  it('refreshes existing edges when their author becomes reachable', () => {
    const graph = new SocialGraph(key(0));
    graph.addFollower(key(1), key(2));
    graph.addFollower(key(0), key(1));
    graph.addFollower(key(1), key(2));
    expect(graph.getFollowDistance(key(2))).toBe(2);
  });

  it('rejects invalid batch sizes and recovers from a failed calculation', async () => {
    const graph = chain(10);
    await expect(graph.recalculateFollowDistances(0)).rejects.toThrow('batchSize');
    await expect(graph.recalculateFollowDistances(100, 1000, () => {
      throw new Error('logger failed');
    })).rejects.toThrow('logger failed');
    await graph.recalculateFollowDistances(100, 1000, () => {});
    expect(graph.getFollowDistance(key(9))).toBe(9);
  });

  it('coalesces a burst of requests while yielding between batches', async () => {
    vi.useFakeTimers();
    const graph = chain(30);
    const logs: string[] = [];
    const recalculate = () => graph.recalculateFollowDistances(5, 1000, message => logs.push(message));
    const first = recalculate();
    await Promise.resolve();
    expect(vi.getTimerCount()).toBeGreaterThan(0);
    const pending = Array.from({length: 20}, recalculate);
    await vi.runAllTimersAsync();
    await Promise.all([first, ...pending]);
    expect(logs.filter(message => message.includes(': start'))).toHaveLength(2);
    expect(graph.getFollowDistance(key(29))).toBe(29);
  });

  it('all overlapping root changes await the final root and graph updates', async () => {
    vi.useFakeTimers();
    const graph = chain(30);
    const pending = graph.recalculateFollowDistances(5, 1000, () => {});
    await Promise.resolve();
    const rootChange = graph.setRoot(key(10));
    const sameRoot = graph.setRoot(key(10));
    let sameRootResolved = false;
    void sameRoot.then(() => { sameRootResolved = true; });
    await Promise.resolve();
    expect(sameRootResolved).toBe(false);
    graph.removeFollower(key(20), key(21));
    graph.addFollower(key(12), key(29));
    await vi.runAllTimersAsync();
    await Promise.all([pending, rootChange, sameRoot]);
    expect(graph.getRoot()).toBe(key(10));
    expect(graph.getFollowDistance(key(10))).toBe(0);
    expect(graph.getFollowDistance(key(0))).toBe(1000);
    expect(graph.getFollowDistance(key(20))).toBe(10);
    expect(graph.getFollowDistance(key(21))).toBe(1000);
    expect(graph.getFollowDistance(key(29))).toBe(3);
    expect(graph.getUsersByFollowDistance(3)).toEqual(new Set([key(13), key(29)]));
  });

  it('includes follow updates that arrive after their author was processed', async () => {
    vi.useFakeTimers();
    const graph = chain(30);
    const pending = graph.recalculateFollowDistances(5, 1000, () => {});
    await Promise.resolve();
    graph.addFollower(key(1), key(25));
    await vi.runAllTimersAsync();
    await pending;
    expect(graph.getFollowDistance(key(29))).toBe(6);
    expect(graph.getUsersByFollowDistance(6)).toEqual(new Set([key(6), key(29)]));
  });
});
