import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import path from 'node:path';

const rustDir = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const script = path.join(rustDir, 'scripts', 'publish.sh');

test('publish plan lists crates in dependency order', () => {
  const output = execFileSync(script, ['--plan'], {
    cwd: rustDir,
    encoding: 'utf8',
  });

  const crates = output
    .trim()
    .split('\n')
    .filter(Boolean)
    .filter((line) => line.startsWith('nostr-social-'));

  assert.deepEqual(crates, [
    'nostr-social-graph',
    'nostr-social-graph-heed',
    'nostr-social-memory',
  ]);
});
