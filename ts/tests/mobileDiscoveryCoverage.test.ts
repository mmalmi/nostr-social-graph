import { readFile } from 'node:fs/promises';
import { describe, expect, it } from 'vitest';
import { SocialGraph } from '../src/SocialGraph';
import { fromBinary } from '../src/SocialGraphBinary';
import { checkMobileGraphCoverage, DEFAULT_GRAPH_SNAPSHOT } from '../src/hashtreeSnapshot';
import { loadGraphBootstrap } from '../src/publicGraphSync';
import { verifyMobileGraphSnapshot, type MobileGraphReceipt } from '../src/mobileGraphSnapshot';
import { namedGraphFixture } from './helpers/namedGraph';

const root = '1'.repeat(64);
async function mobile(users: number) {
  const graph = new SocialGraph(root);
  for (let i = 0; i < users - 1; i++) graph.addFollower(root, i.toString(16).padStart(64, '0'));
  return graph.toBinary();
}

describe('signed mobile discovery quality', () => {
  it('applies the production floor to both public projections and honors stricter consumer floors', () => {
    const {publisher, treeName} = DEFAULT_GRAPH_SNAPSHOT;
    expect(() => checkMobileGraphCoverage(1000, publisher, treeName)).toThrow(/coverage/);
    expect(() => checkMobileGraphCoverage(9999, publisher, treeName, 0)).toThrow(/coverage/);
    expect(() => checkMobileGraphCoverage(10_000, publisher, treeName)).not.toThrow();
    expect(() => checkMobileGraphCoverage(10_000, publisher, treeName, 12_000)).toThrow(/coverage/);
    expect(() => checkMobileGraphCoverage(1000, publisher, 'local-community')).not.toThrow();
  });

  it('rejects an intact signed 1,000-user graph below the requested coverage, while allowing a small custom network', async () => {
    const f = await namedGraphFixture(root, await mobile(1000));
    try {
      const request = {root, snapshot: f.options};
      await expect(loadGraphBootstrap({...request, minUsers: 10_000})).rejects.toThrow(/coverage/);
      expect((await loadGraphBootstrap(request)).size().users).toBe(1000);
    } finally { await f.close(); }
  });

  it('rejects a signed file containing extra bytes after a complete graph', async () => {
    const bytes = await mobile(5);
    const f = await namedGraphFixture(root, Uint8Array.from([...bytes, 0]));
    try {
      await expect(loadGraphBootstrap({root, snapshot: f.options})).rejects.toThrow(/trailing/i);
    } finally { await f.close(); }
  });

  it('rejects mobile follow rows pointing outside the identity table', async () => {
    // One identity, one follow list with a target absent from the identity table.
    const bytes = Uint8Array.of(4, 1, 0, 0, ...Array(32).fill(0x11), 1, 0, 1, 0, 1, 42, 0);
    await expect(fromBinary(root, bytes, true)).rejects.toThrow(/mobile graph list/);
  });

  it('accepts the shipped public graph and its signed discovery coverage', async () => {
    const receipt: MobileGraphReceipt = JSON.parse(await readFile(new URL('../data/socialGraph.json', import.meta.url), 'utf8'));
    const bytes = new Uint8Array(await readFile(new URL('../data/socialGraph.bin', import.meta.url)));
    expect(receipt.source.publisher).toBe(DEFAULT_GRAPH_SNAPSHOT.publisher);
    expect((await verifyMobileGraphSnapshot(bytes, receipt)).size().users).toBeGreaterThanOrEqual(10_000);
  });
});
