// Usage: node --expose-gc --import tsx scripts/benchmarkRanking.ts SNAPSHOT ROOT
// Only reads a local snapshot; no network, storage cache, or graph changes.
import {readFileSync} from 'node:fs';
import {performance} from 'node:perf_hooks';
import {SocialGraph} from '../src/SocialGraph';
import {createRankingSnapshot,calculateRanking} from '../src/ranking';
const [file,root] = process.argv.slice(2);
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
console.log(JSON.stringify({nodes:snapshot.keys.length,follows:snapshot.targets.length,
  mutes:snapshot.muteTargets.length,preparationMs,preparedArrayBytes:snapshot.arrayBytes,
  preparedHeapDelta:prepared.heapUsed-before.heapUsed,gcEnabled:!!global.gc,
  persistentCacheBytes:0,results},null,2));
