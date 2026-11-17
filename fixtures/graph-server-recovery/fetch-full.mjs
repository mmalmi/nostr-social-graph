import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
const [pkg, output] = process.argv.slice(2);
const { HashTree, nhashDecode, BLOB_MAX_BYTES } = await import(import.meta.resolve('@hashtree/core', pathToFileURL(pkg)));
const cid = nhashDecode('nhash1qqs9d8nhumxux5merzgks6hy5z3stcraajzk3qm5u6kwz29syp5p48s9yqdhmc0l4ekle6h27p3zk0ue7pxcvs28cheu3rsdm24u4lq7vwpl668qn0n');
const root = '569e77e6cdc353791891686ae4a0a305e07dec85688374e6ace128b020681a9e';
const expected = { size: 55750905, sha256: 'aeae583ae888f7fec2052f678ff30ae2b031ed4e8d77d0fe0c644e35c2bd6fc0' };
assert.equal(Buffer.from(cid.hash).toString('hex'), root);
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
const signal = AbortSignal.timeout(240000);
const tree = new HashTree({ store: {
  async get(bytes) {
    const hash = Buffer.from(bytes).toString('hex');
    const response = await fetch('https://cdn.iris.to/' + hash + '.bin', { signal, credentials: 'omit', redirect: 'error' });
    assert(response.ok, 'Missing immutable public block ' + hash);
    const parts = []; let size = 0;
    for await (const part of response.body) {
      size += part.length; assert(size <= BLOB_MAX_BYTES, 'Unexpected block length'); parts.push(part);
    }
    const data = Buffer.concat(parts, size);
    assert.equal(digest(data), hash); return data;
  },
}});
const entries = await tree.listDirectory(cid, signal);
const directory = new Map(entries.map(entry => [entry.name, entry]));
assert.equal(directory.size, entries.length);
const manifestEntry = directory.get('manifest.json'); assert(manifestEntry.size <= 65536);
const manifest = JSON.parse(Buffer.from(await tree.readFile(manifestEntry.cid, { maxBytes:65536,signal })).toString('utf8'));
assert.deepEqual(manifest.files['social-graph-v4.bin'], expected);
const member = directory.get('social-graph-v4.bin'); assert.equal(member.size, expected.size);
const data = await tree.readFile(member.cid, { maxBytes:expected.size,signal });
assert.equal(data.length,expected.size); assert.equal(digest(data),expected.sha256);
await writeFile(output, data, {flag:'wx'});
