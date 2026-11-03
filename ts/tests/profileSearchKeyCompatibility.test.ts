import { decryptObject, encryptObject, sha256 } from '@hashtree/core';
import { afterEach, expect, it, vi } from 'vitest';
import { fromHex } from '../scripts/hashtreeAdapter';
import { InMemoryStore } from '../scripts/hashtreeStore';
import { deserializeCid, ProfileSearchIndex, serializeCid } from '../scripts/profileSearchIndex';
import { pushProfileSearchIndex } from '../scripts/publishProfileSearchIndex';
// Captured from @hashtree/index 0.1.13 and @hashtree/core 0.3.1 before upgrading.
import legacy from './legacy-profile-index.json';

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

it.each(['legacy-chk', 'chk-v1', 'rnd-v1'] as const)(
  'opens a persisted %s profile index, publishes it and searches it in the demo',
  async mode => {
    const source = new InMemoryStore();
    for (const [hash, blob] of Object.entries(legacy.blocks)) {
      await source.put(fromHex(hash), Buffer.from(blob, 'base64'));
    }
    let root = deserializeCid(legacy.root);
    if (mode === 'chk-v1') {
      root = await new ProfileSearchIndex(source).indexProfile(root, { pubKey: 'b', name: 'Updated' });
      expect(root.key).toHaveLength(33);
      expect(root.key![0]).toBe(1);
    } else if (mode === 'rnd-v1') {
      const plaintext = await decryptObject((await source.get(root.hash))!, root.key!);
      const { ciphertext, key } = await encryptObject(plaintext, mode);
      root = { hash: await sha256(ciphertext), key };
      await source.put(root.hash, ciphertext);
    }

    const published = new InMemoryStore();
    const restored = deserializeCid(JSON.parse(JSON.stringify(serializeCid(root))));
    expect(restored).toEqual(root);
    const { nhash, stats } = await pushProfileSearchIndex({ root: restored, sourceStore: source, targetStore: published });
    expect(stats.failed).toBe(0);
    if (mode === 'legacy-chk') expect(nhash).toBe(legacy.nhash);

    vi.stubEnv('VITE_PROFILE_SEARCH_INDEX', nhash);
    vi.stubEnv('VITE_BLOSSOM_SERVERS', 'https://blossom.test');
    vi.stubGlobal('fetch', async (url: string) => {
      const match = /^https:\/\/blossom\.test\/([0-9a-f]{64})\.bin$/.exec(url);
      const blob = match && await published.get(fromHex(match[1]));
      return blob ? new Response(Buffer.from(blob)) : new Response(null, { status: 404 });
    });
    vi.resetModules();
    const { searchProfiles } = await import('../examples/src/utils/profileSearch');
    expect((await searchProfiles('legacy')).map(hit => hit.item.pubKey)).toEqual(['a']);
    if (mode === 'chk-v1') {
      expect((await searchProfiles('updated')).map(hit => hit.item.pubKey)).toEqual(['b']);
    }
  },
);

it.each(['', 'ff' + '00'.repeat(32), 'zz'.repeat(32)])('rejects an invalid persisted key %j', key => {
  expect(() => deserializeCid({ ...legacy.root, key })).toThrow();
});
