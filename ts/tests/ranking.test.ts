import {describe, expect, it} from 'vitest';
import {SocialGraph} from '../src/SocialGraph';
import {calculateRanking, createRankingSnapshot} from '../src/ranking';
const key = (n: number) => n.toString(16).padStart(64, '0');
function graph(edges: number[][]) {
  const g = new SocialGraph(key(0));
  for (const [a, b] of edges) g.addFollower(key(a), key(b));
  return g;
}
describe('optional ranking', () => {
  it('returns finite normalized PageRank, including dangling and isolated roots', () => {
    const snapshot = createRankingSnapshot(graph([[0,1],[0,2],[1,3]]));
    const result = calculateRanking(snapshot);
    expect(result.converged).toBe(true);
    expect(snapshot.keys.reduce((sum, k) => sum + result.get(k), 0)).toBeCloseTo(1, 10);
    expect(result.get(key(1))).toBeGreaterThan(result.get(key(3)));
    expect(result.get(key(1))).toBeCloseTo(result.get(key(2)), 12);
    expect(calculateRanking(createRankingSnapshot(graph([]))).get(key(0))).toBe(1);
  });
  it('more independently seeded routes give more support for both algorithms', () => {
    const g = graph([[0,1],[0,2],[0,3],[1,4],[1,5],[2,5],[1,6],[2,6],[3,6]]);
    for (const algorithm of ['pagerank','graperank'] as const) {
      const result = calculateRanking(createRankingSnapshot(g), {algorithm});
      expect(result.get(key(6))).toBeGreaterThan(result.get(key(5)));
      expect(result.get(key(5))).toBeGreaterThan(result.get(key(4)));
    }
  });
  it('PageRank divides influence instead of multiplying it across a large swarm', () => {
    const total = (size: number, cycle: boolean) => {
      const edges = [[0,1],[0,2]];
      for (let i = 0; i < size; i++) { edges.push([1,10+i]); if (cycle) edges.push([10+i,10+(i+1)%size]); }
      const r = calculateRanking(createRankingSnapshot(graph(edges)));
      return Array.from({length:size}, (_, i) => r.get(key(10+i))).reduce((a,b) => a+b,0);
    };
    expect(total(1000,false)).toBeCloseTo(total(1,false), 9);
    expect(total(1000,true)).toBeLessThan(.26);
  });
  it('documents that GrapeRank per-profile caps do not conserve group influence', () => {
    const edges = [[0,1]];
    for (let i = 10; i < 30; i++) { edges.push([1,i]); for (let j = 10; j < 30; j++) if (i !== j) edges.push([i,j]); }
    const r = calculateRanking(createRankingSnapshot(graph(edges)), {algorithm:'graperank'});
    const scores = Array.from({length:20}, (_,i) => r.get(key(i+10)));
    expect(scores.every(s => s >= 0 && s <= 1)).toBe(true);
    expect(scores.reduce((a,b) => a+b,0)).toBeGreaterThan(10);
  });
  it('root mutes remove an entire exclusive branch without mutating source lists', () => {
    const g = graph([[0,1],[0,2],[1,3]]), d = g.getInternalData();
    d.mutedByUser.set(d.ids.id(key(0)), new Set([d.ids.id(key(1))]));
    const snapshot = createRankingSnapshot(g);
    expect(snapshot.keys).toEqual([key(0),key(2)]);
    expect(g.getFollowedByUser(key(0)).has(key(1))).toBe(true);
    expect(calculateRanking(snapshot).get(key(3))).toBe(0);
  });
  it('trusted mutes reduce GrapeRank and override an author’s positive edge', () => {
    const g = graph([[0,1],[0,2],[1,3],[2,3]]), d = g.getInternalData();
    const before = calculateRanking(createRankingSnapshot(g), {algorithm:'graperank'}).get(key(3));
    d.mutedByUser.set(d.ids.id(key(2)),new Set([d.ids.id(key(3))]));
    expect(calculateRanking(createRankingSnapshot(g),{algorithm:'graperank'}).get(key(3))).toBeLessThan(before);
  });
  it('snapshots survive source edits and lookup of unknown keys stays read-only', () => {
    const g = graph([[0,1],[1,2]]), snap = createRankingSnapshot(g), result = calculateRanking(snap);
    const before = result.get(key(2));
    expect(result.get(key(999))).toBe(0); expect(g.getInternalData().ids.has(key(999))).toBe(false);
    g.removeFollower(key(1),key(2));
    expect(result.get(key(2))).toBe(before);
    expect(calculateRanking(createRankingSnapshot(g)).get(key(2))).toBe(0);
  });
  it('disconnected cycles cannot bootstrap authority and self-links add none', () => {
    const g = graph([[0,1],[2,3],[3,2],[1,1]]);
    for (const algorithm of ['pagerank','graperank'] as const) {
      const r = calculateRanking(createRankingSnapshot(g),{algorithm});
      expect(r.get(key(2))).toBe(0); expect(r.get(key(3))).toBe(0);
      expect(r.get(key(1))).toBeCloseTo(calculateRanking(createRankingSnapshot(graph([[0,1]])),{algorithm}).get(key(1)),12);
    }
  });
  it('bounds iterations, total work, and graph preparation without silent truncation', () => {
    const g = graph([[0,1],[1,2]]), snap = createRankingSnapshot(g);
    expect(() => createRankingSnapshot(g,{maxNodes:2})).toThrow('maxNodes');
    expect(() => createRankingSnapshot(g,{maxEdges:1})).toThrow('maxEdges');
    expect(() => calculateRanking(snap,{maxWork:1})).toThrow('maxWork');
    const bounded = calculateRanking(snap,{maxWork:10});
    expect(bounded.work).toBeLessThanOrEqual(10); expect(bounded.converged).toBe(false);
    expect(calculateRanking(snap,{maxIterations:1}).iterations).toBe(1);
    for (const damping of [0,1,NaN,Infinity]) expect(() => calculateRanking(snap,{damping})).toThrow('damping');
  });
});
