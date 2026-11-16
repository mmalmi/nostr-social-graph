import { describe, expect, it } from 'vitest';
import { SocialGraph } from '../src/SocialGraph';
import { openGraphSnapshot } from '../src/hashtreeSnapshot';
import { loadGraphBootstrap } from '../src/publicGraphSync';
import { namedGraphFixture } from './helpers/namedGraph';

const root = '1'.repeat(64);
async function fixture() {
  const graph = new SocialGraph(root);
  graph.handleEvent({pubkey: root, kind: 3, tags: [['p', '2'.repeat(64)]], id: 'a'.repeat(64), created_at: 42, content: '', sig: ''});
  graph.handleEvent({pubkey: root, kind: 10000, tags: [['p', '3'.repeat(64)]], id: 'b'.repeat(64), created_at: 43, content: '', sig: ''});
  graph.markListIncomplete(root, 10000);
  return namedGraphFixture(root, await graph.toBinary());
}
describe('direct named graph snapshot', () => {
  it('reads only the requested mobile root and retains winning list IDs and completeness', async () => {
    const f = await fixture();
    try {
      const graph = await loadGraphBootstrap({root, snapshot: f.options});
      expect(graph.getListRevision(root, 3)?.id).toBe('a'.repeat(64));
      expect(graph.getListRevision(root, 10000)).toMatchObject({id:'b'.repeat(64), complete:false});
      expect(f.filters).toHaveLength(1);
      for (const file of f.uploaded.filter(file => !file.name.startsWith('mobile-'))) {
        expect(f.requests).not.toContain(file.hash);
      }
      await expect(loadGraphBootstrap({root:'f'.repeat(64), snapshot:f.options})).rejects.toThrow('no mobile projection');
    } finally { await f.close(); }
  });

  it('pins an immutable generation, enforces byte limits before transfer and rejects a changed digest', async () => {
    const f = await fixture();
    try {
      const original = await openGraphSnapshot(f.options);
      const requests = f.requests.length;
      await expect(original.readFile('profile-data.json', {maxBytes: 4})).rejects.toThrow('size');
      expect(f.requests).toHaveLength(requests);
      f.manifest.files['profile-data.json'].sha256 = 'f'.repeat(64);
      await f.resign();
      expect(await original.readFile('profile-data.json')).toEqual(f.profile);
      const newer = await openGraphSnapshot(f.options);
      expect(newer.directoryHash).not.toBe(original.directoryHash);
      await expect(newer.readFile('profile-data.json')).rejects.toThrow('digest');
    } finally { await f.close(); }
  });

  it('rejects an invalid signature before requesting any content', async () => {
    const f = await fixture();
    try {
      f.corruptSignature();
      await expect(openGraphSnapshot(f.options)).rejects.toThrow();
      expect(f.requests).toHaveLength(0);
    } finally { await f.close(); }
  });
});
