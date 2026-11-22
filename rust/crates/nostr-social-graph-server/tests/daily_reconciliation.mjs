// Paired release-binary test, or a complete production-sized synthetic daily sweep.
// The publication fixture supplies isolated relay, Blossom, signer and readback.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const {values} = parseArgs({options: {
  baseline: {type: 'string'}, candidate: {type: 'string'}, dependencies: {type: 'string'},
  output: {type: 'string'}, htree: {type: 'string'}, 'legacy-htree': {type: 'string'},
  'full-size': {type: 'boolean'},
}});
const full = Boolean(values['full-size']);
for (const arg of ['candidate', 'dependencies', 'output', 'htree', 'legacy-htree', ...(full ? [] : ['baseline'])]) {
  assert(values[arg], 'Missing --' + arg);
}
const out = path.resolve(values.output);
assert(!existsSync(out), 'Use a new evidence directory');
mkdirSync(out, {recursive: true});
const harness = fileURLToPath(new URL('./hashtree_publication.mjs', import.meta.url));
const result = {passed: false, mode: full ? 'full-size' : 'paired'};
try {
  for (const label of full ? ['candidate'] : ['baseline', 'candidate']) {
    const output = path.join(out, label + '.json');
    const child = spawnSync(process.execPath, [harness, path.resolve(values[label]),
      '--dependencies', path.resolve(values.dependencies), '--htree', path.resolve(values.htree),
      '--legacy-htree', path.resolve(values['legacy-htree']), '--output', output,
      '--daily-trial', full ? 'full-size' : 'compact'], {stdio: 'inherit'});
    assert.ifError(child.error);
    assert.equal(child.status, 0, label + ' trial failed, signal ' + child.signal);
    result[label] = JSON.parse(readFileSync(output, 'utf8'));
    assert(result[label].passed);
  }
  if (full) {
    assert(result.candidate.signedReadMs < 7200000, 'Full signed cold read exceeded two hours');
    assert.equal(result.candidate.verified.users, 318421);
  } else {
    assert.deepEqual(result.candidate.verified, result.baseline.verified, 'Different resulting relationships');
    assert.deepEqual(result.candidate.distance.map(d => d.authorsVisited),
      result.baseline.distance.map(d => d.authorsVisited), 'Different input coverage');
    assert.equal(result.baseline.maxInFlight, 1, 'Reference binary did not reproduce sequential requests');
    assert(result.candidate.maxInFlight > 1, 'Candidate did not overlap independent batches');
    result.crawlSpeedup = result.baseline.crawlMs / result.candidate.crawlMs;
    assert(result.crawlSpeedup > 1.25, 'No material improvement in the shaped relay sweep');
  }
  result.passed = true;
} finally {
  writeFileSync(path.join(out, 'receipt.json'), JSON.stringify(result, null, 2) + '\n');
}
