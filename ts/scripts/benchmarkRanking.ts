// Usage: node --expose-gc --import tsx scripts/benchmarkRanking.ts SNAPSHOT ROOT [BASELINE_MODULE]
// Only reads a local snapshot; no network, storage cache, or graph changes.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {performance} from 'node:perf_hooks';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
import {SocialGraph} from '../src/SocialGraph';
import {createRankingSnapshot,calculateRanking} from '../src/ranking';
const [file,root,baselineFile] = process.argv.slice(2);
if (!file || !root) throw new Error('Provide snapshot path and public root');
const graph = await SocialGraph.fromBinary(root,readFileSync(file));
global.gc?.();
const before = process.memoryUsage();
const start = performance.now(), snapshot = createRankingSnapshot(graph);
const preparationMs = performance.now() - start;
global.gc?.();
const prepared = process.memoryUsage();
const results = [];
for (const algorithm of ['pagerank','graperank'] as const) {
  const samples = [];
  let result = calculateRanking(snapshot,{algorithm});
  for (let i=0;i<7;i++) {
    const at = performance.now(); result = calculateRanking(snapshot,{algorithm});
    samples.push(performance.now()-at);
  }
  const sorted = [...samples].sort((a,b)=>a-b);
  results.push({algorithm,medianMs:sorted[3],maxMs:sorted[6],iterations:result.iterations,
    converged:result.converged,work:result.work,workingArrayBytes:result.workingArrayBytes});
}

function paired(baseline: () => unknown, candidate: () => unknown) {
  for (let i = 0; i < 4; i++) { baseline(); candidate(); }
  const samples = [[], []] as number[][];
  for (let i = 0; i < 15; i++) {
    for (const version of i % 2 ? [1, 0] : [0, 1]) {
      const at = performance.now();
      (version ? candidate : baseline)();
      samples[version].push(performance.now() - at);
    }
  }
  const median = (values: number[]) => [...values].sort((a, b) => a - b)[7];
  return {baselineMedianMs: median(samples[0]), candidateMedianMs: median(samples[1]),
    pairedMedianRatio: median(samples[1].map((value, i) => value / samples[0][i]))};
}

let comparison;
if (baselineFile) {
  const baseline = await import(pathToFileURL(resolve(baselineFile)).href) as {
    createRankingSnapshot: typeof createRankingSnapshot;
    calculateRanking: typeof calculateRanking;
  };
  const previous = baseline.createRankingSnapshot(graph);
  assert.deepEqual(snapshot, previous, 'Snapshot differs from baseline');
  const calculations = [];
  for (const algorithm of ['pagerank', 'graperank'] as const) {
    const old = baseline.calculateRanking(previous, {algorithm}), next = calculateRanking(snapshot, {algorithm});
    for (const field of ['iterations', 'converged', 'residual', 'work', 'workingArrayBytes', 'directSeed'] as const) {
      assert.equal(next[field], old[field], `Result metadata differs: ${field}`);
    }
    for (const key of snapshot.keys) assert.equal(next.get(key), old.get(key), 'Score differs from baseline');
    calculations.push({algorithm, ...paired(() => baseline.calculateRanking(snapshot, {algorithm}),
      () => calculateRanking(snapshot, {algorithm}))});
  }
  comparison = {exactParity: true, preparation: paired(() => baseline.createRankingSnapshot(graph),
    () => createRankingSnapshot(graph)), calculations};
}
console.log(JSON.stringify({nodes:snapshot.keys.length,follows:snapshot.targets.length,
  mutes:snapshot.muteTargets.length,preparationMs,preparedArrayBytes:snapshot.arrayBytes,
  preparedHeapDelta:prepared.heapUsed-before.heapUsed,gcEnabled:!!global.gc,
  persistentCacheBytes:0,results,comparison},null,2));
