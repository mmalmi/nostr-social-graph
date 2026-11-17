"""Verify full payload equivalence and bounded repeated-cycle memory against accepted evidence."""
import json
from pathlib import Path
import sys

baseline, candidate, output = map(Path, sys.argv[1:])
def read(path):
    return json.loads(path.read_text())
old = read(baseline/'baseline-receipt.json')
new = read(candidate/'receipt.json')
old_memory = read(baseline/'baseline-crawler-memory.json')
new_memory = read(candidate/'crawler-memory.json')
new_restart = read(candidate/'restart-memory.json')
assert old['passed'] and new['passed'] and not new_memory.get('risk') and not new_restart.get('risk')
for field in ['seedSha256','seedBytes','primaryRoot','fixtureEventIds','checkpoint','fileHashes','checks',
              'coldNamedRead','oldImmutableRead','signedPublisher','upstreamWrites']:
    assert new[field] == old[field], field
assert new['cycles'] >= 5 and new['partials'] >= 2*new['cycles']
retained = [sample['anon'] for sample in new_memory['settled'] if sample['cycle'] > new['changedAt']]
assert len(retained) >= 3
peak = max(stage['peak'].get('hwm', 0) for stage in new_memory['stages'])
old_peak = max(stage['peak'].get('hwm', 0) for stage in old_memory['stages'])
restart = next(stage['peak']['hwm'] for stage in new_restart['stages'] if stage['name']=='restart-startup')
# Preserve the measured single-graph working set. The allowance covers allocator
# layout and the new independently scheduled, bounded profile snapshot buffer.
gates = dict(semanticPayload=True, repeatedCycles=True,
             memoryBound=peak <= 3*2**30,
             restartBound=restart <= 3*2**30,
             retainedStable=max(retained)-min(retained) <= max(48*2**20,max(retained)*.10))
report = dict(passed=all(gates.values()),gates=gates,priorPeakBytes=old_peak,peakBytes=peak,
              restartBytes=restart,retainedBytes=retained,priorBinary=old['binarySha256'],
              candidateBinary=new['binarySha256'],cycles=new['cycles'],fileHashes=new['fileHashes'])
output.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report))
assert report['passed'], gates
