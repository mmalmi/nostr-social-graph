/** Optional, synchronous graph scoring. Import from nostr-social-graph/ranking.
 * No timers, networking, storage, or changes to the supplied graph.
 * Run in a worker for large inputs; cache by root and graph revision in the app.
 */
export interface RankingGraph {
  getRoot(): string;
  getInternalData(): {
    ids: { has(key: string): boolean; id(key: string): number };
    str(id: number): string;
    followedByUser: ReadonlyMap<number, ReadonlySet<number>>;
    mutedByUser: ReadonlyMap<number, ReadonlySet<number>>;
  };
}
export interface RankingLimits { maxNodes?: number; maxEdges?: number }
export interface RankingOptions {
  algorithm?: 'pagerank' | 'graperank';
  /** PageRank continuation probability; remaining mass restarts at the root. */
  damping?: number;
  /** GrapeRank discount for opinions outside the root. */
  attenuation?: number;
  /** GrapeRank confidence = 1 - rigor ** evidenceWeight. */
  rigor?: number;
  tolerance?: number;
  maxIterations?: number;
  /** Total node + edge visits across iterations, including negative edges. */
  maxWork?: number;
}
export interface RankingSnapshot {
  readonly root: string;
  readonly keys: readonly string[];
  readonly indices: ReadonlyMap<string, number>;
  readonly offsets: Uint32Array;
  readonly targets: Uint32Array;
  readonly muteOffsets: Uint32Array;
  readonly muteTargets: Uint32Array;
  readonly arrayBytes: number;
}
export interface RankingResult {
  readonly algorithm: 'pagerank' | 'graperank';
  readonly root: string;
  readonly iterations: number;
  readonly converged: boolean;
  readonly residual: number;
  readonly nodes: number;
  readonly edges: number;
  readonly work: number;
  /** Typed-array working set, excluding keys, Maps and the caller's graph. */
  readonly workingArrayBytes: number;
  /** Root's direct contribution to one follow, useful as a reference scale. */
  readonly directSeed: number;
  /** Unknown and excluded keys return zero, without registering graph IDs. */
  get(pubkey: string): number;
}

function positiveInteger(value: number, name: string, ceiling = Number.MAX_SAFE_INTEGER) {
  if (!Number.isSafeInteger(value) || value < 1 || value > ceiling) throw new RangeError(name);
  return value;
}
function fraction(value: number, name: string) {
  if (!Number.isFinite(value) || value <= 0 || value >= 1) throw new RangeError(name);
  return value;
}

/** Copy reachable follow/mute edges into bounded compact arrays. Root mutes
 * exclude a profile and its outgoing influence; other mutes override that
 * author's follows. Unknown/unreachable mute targets never gain influence.
 * Incomplete snapshots produce incomplete scores. Limits reject, not truncate.
 */
export function createRankingSnapshot(graph: RankingGraph, limits: RankingLimits = {}): RankingSnapshot {
  const maxNodes = positiveInteger(limits.maxNodes ?? 250_000, 'maxNodes', 0xffffffff);
  const maxEdges = positiveInteger(limits.maxEdges ?? 2_000_000, 'maxEdges', 0xffffffff);
  const root = graph.getRoot(), data = graph.getInternalData();
  if (!data.ids.has(root)) throw new Error('Ranking root is absent');
  const rootId = data.ids.id(root), blocked = data.mutedByUser.get(rootId);
  const ids = [rootId], positions = new Map([[rootId, 0]]);
  let inspected = 0;
  for (let head = 0; head < ids.length; head++) {
    const owner = ids[head], muted = data.mutedByUser.get(owner);
    for (const target of data.followedByUser.get(owner) ?? []) {
      if (++inspected > maxEdges) throw new RangeError('maxEdges');
      if (target === owner || (target !== rootId && blocked?.has(target)) || muted?.has(target)) continue;
      if (!positions.has(target)) {
        if (ids.length >= maxNodes) throw new RangeError('maxNodes');
        positions.set(target, ids.length); ids.push(target);
      }
    }
    inspected += muted?.size ?? 0;
    if (inspected > maxEdges) throw new RangeError('maxEdges');
  }
  const offsets = new Uint32Array(ids.length + 1), muteOffsets = new Uint32Array(ids.length + 1);
  // Two passes avoid retaining nested arrays or growing typed-array buffers.
  const visit = (i: number, follows: (target: number) => void, mutes: (target: number) => void) => {
    const id = ids[i], muted = data.mutedByUser.get(id);
    for (const target of data.followedByUser.get(id) ?? []) {
      const position = positions.get(target);
      if (target !== id && position !== undefined && !muted?.has(target)) follows(position);
    }
    for (const target of muted ?? []) {
      const position = positions.get(target);
      if (target !== id && position !== undefined) mutes(position);
    }
  };
  for (let i = 0; i < ids.length; i++) {
    offsets[i + 1] = offsets[i]; muteOffsets[i + 1] = muteOffsets[i];
    visit(i, () => offsets[i + 1]++, () => muteOffsets[i + 1]++);
  }
  const targets = new Uint32Array(offsets[ids.length]), muteTargets = new Uint32Array(muteOffsets[ids.length]);
  for (let i = 0; i < ids.length; i++) {
    let f = offsets[i], m = muteOffsets[i];
    visit(i, target => { targets[f++] = target; }, target => { muteTargets[m++] = target; });
  }
  const keys = ids.map(id => data.str(id));
  return {root, keys, indices: new Map(keys.map((key, i) => [key, i])), offsets, targets, muteOffsets, muteTargets,
    arrayBytes: offsets.byteLength + targets.byteLength + muteOffsets.byteLength + muteTargets.byteLength};
}

/** Personalized PageRank or a GrapeRank-style capped weighted average.
 * PageRank divides an author's influence among their follow edges and sends
 * dangling mass back to the root. GrapeRank does not conserve aggregate mass:
 * a cap per profile is not a cap on a group of profiles. Neither proves identity.
 * GrapeRank's formula follows the MIT reference credited in ranking-NOTICE.md;
 * this implementation uses simultaneous updates and global convergence.
 */
export function calculateRanking(snapshot: RankingSnapshot, options: RankingOptions = {}): RankingResult {
  const algorithm = options.algorithm ?? 'pagerank';
  if (algorithm !== 'pagerank' && algorithm !== 'graperank') throw new RangeError('algorithm');
  const damping = fraction(options.damping ?? .5, 'damping');
  const attenuation = fraction(options.attenuation ?? .5, 'attenuation');
  const rigor = fraction(options.rigor ?? .5, 'rigor');
  const tolerance = options.tolerance ?? 1e-8;
  if (!Number.isFinite(tolerance) || tolerance <= 0 || tolerance >= 1) throw new RangeError('tolerance');
  const maxIterations = positiveInteger(options.maxIterations ?? 64, 'maxIterations', 10_000);
  const maxWork = positiveInteger(options.maxWork ?? 64_000_000, 'maxWork');
  const {keys, indices, offsets, targets, muteOffsets, muteTargets} = snapshot;
  const n = keys.length, perIteration = n + targets.length + (algorithm === 'graperank' ? muteTargets.length : 0);
  if (perIteration > maxWork) throw new RangeError('maxWork');
  const limit = Math.min(maxIterations, Math.floor(maxWork / perIteration));
  let values = new Float64Array(n), next = new Float64Array(n);
  const weights = algorithm === 'graperank' ? new Float64Array(n) : null;
  values[0] = 1;
  let iterations = 0, residual = Infinity;
  while (iterations < limit && residual > tolerance) {
    next.fill(0); weights?.fill(0);
    if (algorithm === 'pagerank') {
      next[0] = 1 - damping;
      for (let i = 0; i < n; i++) {
        const count = offsets[i + 1] - offsets[i], mass = damping * values[i];
        if (!count) { next[0] += mass; continue; }
        const contribution = mass / count;
        for (let j = offsets[i]; j < offsets[i + 1]; j++) next[targets[j]] += contribution;
      }
    } else {
      for (let i = 0; i < n; i++) {
        const contribution = values[i] * (i === 0 ? 1 : attenuation);
        if (!contribution) continue;
        for (let j = offsets[i]; j < offsets[i + 1]; j++) {
          const target = targets[j]; next[target] += contribution; weights![target] += contribution;
        }
        for (let j = muteOffsets[i]; j < muteOffsets[i + 1]; j++) weights![muteTargets[j]] += contribution;
      }
      for (let i = 1; i < n; i++) next[i] = weights![i] ? next[i] / weights![i] * -Math.expm1(Math.log(rigor) * weights![i]) : 0;
      next[0] = 1;
    }
    residual = 0;
    for (let i = 0; i < n; i++) residual += Math.abs(next[i] - values[i]);
    [values, next] = [next, values]; iterations++;
  }
  const directCount = offsets[1] - offsets[0];
  return {algorithm, root: snapshot.root, nodes: n, edges: targets.length + muteTargets.length,
    iterations, converged: residual <= tolerance, residual, work: iterations * perIteration,
    workingArrayBytes: snapshot.arrayBytes + values.byteLength + next.byteLength + (weights?.byteLength ?? 0),
    directSeed: algorithm === 'pagerank' ? (directCount ? (1 - damping) * damping / directCount : 0) : 1 - rigor,
    get: pubkey => { const index = indices.get(pubkey); return index === undefined ? 0 : values[index]; }};
}
