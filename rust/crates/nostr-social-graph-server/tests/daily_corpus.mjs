// Generated Nostr data for the isolated daily full-reconciliation benchmark.
// Keep only keys and signed event headers in memory. Rebuild tag bodies on demand.
import assert from 'node:assert/strict';
import { createECDH, createHash } from 'node:crypto';
import { closeSync, openSync, writeSync } from 'node:fs';

const OLD = 1_700_000_000;
const NEW = OLD + 120;
const FULL = [
  // Reachable users, follow heads, mute heads, extra follows within that layer.
  [1, 1, 0, 0],
  [420, 360, 56, 180],
  [31_000, 23_000, 1_800, 480],
  [164_000, 27_500, 5_000, 158],
  [123_000, 10_000, 4_000, 55],
];
const SMALL = [
  [1, 1, 0, 0], [48, 36, 4, 10], [192, 121, 14, 30],
  [320, 90, 28, 18], [512, 9, 4, 6],
];

const hash = data => createHash('sha256').update(data).digest('hex');
const secret = index => createHash('sha256').update('daily-graph-fixture-v1/key/' + index).digest();

// Spread row owners through the layer while retaining contiguous wholly empty tail batches.
function selected(local, count, span) {
  return local < span && Math.floor((local + 1) * count / span) > Math.floor(local * count / span);
}

function binaryWriter(path) {
  const fd = openSync(path, 'w', 0o600);
  const digest = createHash('sha256');
  const block = Buffer.allocUnsafe(64 * 1024);
  let offset = 0;
  function flush() {
    const bytes = block.subarray(0, offset);
    digest.update(bytes);
    let written = 0;
    while (written < offset) written += writeSync(fd, bytes, written, offset - written);
    offset = 0;
  }
  function byte(n) {
    if (offset === block.length) flush();
    block[offset++] = n;
  }
  return {
    number(n) {
      while (n >= 128) {
        byte(128 + (n % 128));
        n = Math.floor(n / 128);
      }
      byte(n);
    },
    hex(s) {
      if (offset + 32 > block.length) flush();
      assert.equal(block.write(s, offset, 32, 'hex'), 32);
      offset += 32;
    },
    finish() { flush(); closeSync(fd); return digest.digest('hex'); },
  };
}

export async function createDailyCorpus({ fullSize, rootKey, getPublicKey, finalizeEvent, seedPath }) {
  const specs = fullSize ? FULL : SMALL;
  const total = specs.reduce((n, [size]) => n + size, 0);
  const keys = new Array(total);
  const layers = [];
  const positions = new Map();
  const depthByAuthor = new Map();
  const followOrdinal = new Int32Array(total).fill(-1);
  const muteOrdinal = new Int32Array(total).fill(-1);
  const depthAt = new Uint8Array(total);
  const ecdh = createECDH('secp256k1');
  let offset = 0;

  for (const [depth, [size, follows, mutes, extra]] of specs.entries()) {
    const layer = { start: offset, size, follows, mutes, extra };
    layers.push(layer);
    const span = Math.max(follows, mutes, size - Math.floor(size / 8));
    let nf = 0, nm = 0;
    for (let i = 0; i < size; ++i) {
      const idx = layer.start + i;
      const priv = idx === 0 ? rootKey : secret(idx);
      ecdh.setPrivateKey(priv);
      const key = ecdh.getPublicKey(undefined, 'compressed').subarray(1).toString('hex');
      if (idx === 0) assert.equal(key, getPublicKey(rootKey));
      keys[idx] = key;
      positions.set(key, idx);
      depthAt[idx] = depth;
      depthByAuthor.set(key, depth);
      if (selected(i, follows, span)) followOrdinal[idx] = nf++;
      if (selected(i, mutes, span)) muteOrdinal[idx] = nm++;
    }
    assert.equal(nf, follows);
    assert.equal(nm, mutes);
    offset += size;
  }
  assert.equal(positions.size, total);

  function targets(idx, kind) {
    const depth = depthAt[idx], layer = layers[depth], local = idx - layer.start;
    const rows = [];
    if (kind === 10000) {
      // Include active list owners in mute lookups. Every target has a parent
      // one layer closer to the root, so a same-layer mute cannot overmute it.
      const count = 1 + (idx % 8);
      for (let j = 0; rows.length < count; j++) {
        const target = (local * 97 + 17 + j) % layer.size;
        if (target !== local) rows.push(layer.start + target);
      }
      return rows;
    }
    const next = layers[depth + 1];
    if (next) {
      const ordinal = followOrdinal[idx];
      const start = Math.floor(ordinal * next.size / layer.follows);
      const end = Math.floor((ordinal + 1) * next.size / layer.follows);
      for (let j = start; j < end; j++) rows.push(next.start + j);
    }
    // Cross-links add the observed high degree without shortening distances.
    for (let j = 0, added = 0; added < layer.extra; j++) {
      const target = (local * 997 + 13 + j) % layer.size;
      if (target !== local) { rows.push(layer.start + target); added++; }
    }
    return rows;
  }

  const hasHead = (idx, kind) => (kind === 3 ? followOrdinal[idx] : muteOrdinal[idx]) >= 0;
  // Some leaf authors first acquire a list on the measured sweep. All routes
  // connecting layers already exist in the seed.
  const absentFromSeed = (idx, kind) =>
    depthAt[idx] === layers.length - 1 && (kind === 3 ? followOrdinal[idx] : muteOrdinal[idx]) % 13 === 0;
  const changed = (idx, kind) => idx !== 0 && (idx + kind) % 11 === 0;
  const header = (idx, kind, at, rows) => ({
    kind, created_at: at, content: '', tags: rows.map(i => ['p', keys[i]]),
  });
  const metadata = new Map();
  const writer = binaryWriter(seedPath);
  writer.number(4);
  writer.number(keys.length);
  for (let i = 0; i < keys.length; i++) {
    writer.number(i + 1); writer.number(0); writer.hex(keys[i]);
  }
  const summary = {
    layers: specs.map(row => row[0]), users: total, follows: 0, mutes: 0,
    followRevisions: 0, muteRevisions: 0, existingHeads: 0, changedHeads: 0,
    newHeads: 0, emptyAuthors: 0, seedSha256: null,
  };
  const totals = kind => kind === 3 ? ['followRevisions', 'follows'] : ['muteRevisions', 'mutes'];
  for (const kind of [3, 10000]) {
    let seedCount = 0;
    for (let i = 0; i < total; i++) if (hasHead(i, kind) && !absentFromSeed(i, kind)) seedCount++;
    writer.number(seedCount);
    for (let idx = 0; idx < total; idx++) {
      if (!hasHead(idx, kind)) continue;
      const rows = targets(idx, kind), absent = absentFromSeed(idx, kind);
      const updated = changed(idx, kind);
      const at = (absent || updated) ? NEW : OLD;
      const event = finalizeEvent(header(idx, kind, at, rows), idx === 0 ? rootKey : secret(idx));
      assert.equal(event.pubkey, keys[idx]);
      metadata.set(idx * 2 + Number(kind === 10000), { id: event.id, sig: event.sig, at });
      const [headCount, edgeCount] = totals(kind);
      summary[headCount]++;
      summary[edgeCount] += rows.length;
      if (absent) { summary.newHeads++; continue; }
      let id = event.id;
      if (updated) {
        summary.changedHeads++;
        // Drop one optional link in the old row, keeping cross-layer coverage.
        rows.pop();
        const oldTags = rows.map(i => ['p', keys[i]]);
        id = hash(JSON.stringify([0, keys[idx], OLD, kind, oldTags, '']));
      } else {
        summary.existingHeads++;
      }
      writer.number(idx + 1); writer.number(OLD); writer.number(2); writer.hex(id);
      writer.number(rows.length);
      for (const target of rows) writer.number(target + 1);
    }
  }
  summary.seedSha256 = writer.finish();
  const authorsByDepth = layers.map(({ start, size }) => keys.slice(start, start + size));
  const emptyAuthors = new Set(keys.filter((_, i) => !hasHead(i, 3) && !hasHead(i, 10000)));
  summary.emptyAuthors = emptyAuthors.size;

  const cache = new Map();
  function heads(key) {
    const idx = positions.get(key);
    if (idx === undefined) return [];
    if (cache.has(idx)) return cache.get(idx);
    const events = [];
    for (const kind of [3, 10000]) {
      const meta = metadata.get(idx * 2 + Number(kind === 10000));
      if (meta) events.push({
        ...header(idx, kind, meta.at, targets(idx, kind)), pubkey: key, id: meta.id, sig: meta.sig,
      });
    }
    if (cache.size >= 256) cache.delete(cache.keys().next().value);
    cache.set(idx, events);
    return events;
  }

  function verifySnapshot(input) {
    const bytes = Buffer.isBuffer(input) ? input : Buffer.from(input);
    let pos = 0;
    function number() {
      let value = 0, place = 1;
      for (let len = 0; len < 8 && pos < bytes.length; len++) {
        const b = bytes[pos++];
        value += (b & 127) * place;
        if (!(b & 128)) return value;
        place *= 128;
      }
      throw Error('Invalid v4 varint');
    }
    function hex() {
      assert(pos + 32 <= bytes.length);
      const value = bytes.toString('hex', pos, pos + 32);
      pos += 32;
      return value;
    }
    assert.equal(number(), 4);
    assert.equal(number(), total);
    const owners = new Map(), present = new Set();
    for (let i = 0; i < total; i++) {
      const id = number();
      assert.equal(number(), 0);
      const idx = positions.get(hex());
      assert(idx !== undefined && !owners.has(id) && !present.has(idx), 'Missing/duplicate public key');
      owners.set(id, idx); present.add(idx);
    }
    const semantic = createHash('sha256');
    const checked = { users: total, follows: 0, mutes: 0, followRevisions: 0, muteRevisions: 0 };
    for (const kind of [3, 10000]) {
      const [headCount, edgeCount] = totals(kind);
      const n = number();
      assert.equal(n, summary[headCount]);
      const verified = new Set();
      for (let i = 0; i < n; i++) {
        const idx = owners.get(number());
        assert(idx !== undefined && hasHead(idx, kind) && !verified.has(idx), 'Unexpected/duplicate owner');
        const meta = metadata.get(idx * 2 + Number(kind === 10000));
        assert.equal(number(), meta.at);
        assert.equal(number(), 2);
        assert.equal(hex(), meta.id);
        const remaining = new Set(targets(idx, kind));
        const edges = number();
        assert.equal(edges, remaining.size);
        for (let j = 0; j < edges; j++)
          assert(remaining.delete(owners.get(number())), 'Wrong target or duplicate relation');
        checked[edgeCount] += edges;
        verified.add(idx);
      }
      checked[headCount] = verified.size;
      assert.equal(checked[edgeCount], summary[edgeCount]);
      // Event IDs bind the verified timestamp, owner, kind and ordered targets.
      // Use fixture order to compare snapshots with different internal IDs.
      for (let idx = 0; idx < total; idx++) {
        if (verified.has(idx)) semantic.update(metadata.get(idx * 2 + Number(kind === 10000)).id);
      }
    }
    assert.equal(pos, bytes.length);
    return { ...checked, semanticSha256: semantic.digest('hex') };
  }
  return { authorsByDepth, depthByAuthor, heads, verifySnapshot, summary, emptyAuthors };
}
