import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  BINARY_FORMAT_VERSION,
  BINARY_FORMAT_VERSION_V5,
  SocialGraph,
  fromBinary,
  toBinary,
  toBinaryChunks,
} from '../src/index.js';
import type { NostrEvent } from '../src/utils.js';

const key = (n: number) => n.toString(16).padStart(64, '0');
const revId = (n: number) => n.toString(16).padStart(64, 'a');

const event = (author: string, kind: number, created_at: number, targets: string[], id = revId(created_at)): NostrEvent => ({
  pubkey: author,
  kind,
  created_at,
  tags: targets.map(t => ['p', t]),
  content: '',
  id,
  sig: '00'.repeat(64),
});

describe('binary v5 codec and compact internal storage', () => {
  it('defaults to v4 while opt-in v5 round-trips revisions, empty lists, UUIDs, and UTF-8 strings', async () => {
    const root = key(1);
    const bob = key(2);
    const carol = key(3);
    const uuid = '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a';
    const utf8 = 'external:peer:🚀-αβγ';

    const graph = new SocialGraph(root);
    graph.handleEvent(event(root, 3, 1000, [bob, carol]), true);
    graph.handleEvent(event(bob, 3, 1100, []), true);
    graph.handleEvent(event(root, 10000, 1200, [carol]), true);
    graph.handleEvent(event(bob, 10000, 1300, []), true);
    graph.addFollower(carol, uuid);
    graph.addFollower(uuid, utf8);

    const v4 = await toBinary(graph);
    expect(v4[0]).toBe(BINARY_FORMAT_VERSION);

    const v5 = await toBinary(graph, undefined, undefined, undefined, undefined, BINARY_FORMAT_VERSION_V5);
    expect(v5[0]).toBe(BINARY_FORMAT_VERSION_V5);
    expect(v5.length).toBeLessThan(v4.length);

    // Chunked v5 matches single-buffer v5 exactly.
    const chunks: Uint8Array[] = [];
    for await (const chunk of toBinaryChunks(graph, undefined, undefined, undefined, undefined, 5)) {
      chunks.push(chunk);
    }
    const reassembled = Buffer.concat(chunks);
    expect(new Uint8Array(reassembled)).toEqual(v5);

    const restored = await SocialGraph.fromBinary(root, v5);
    expect(restored.getRoot()).toBe(root);
    expect(restored.isFollowing(root, bob)).toBe(true);
    expect(restored.isFollowing(root, carol)).toBe(true);
    expect(restored.isFollowing(carol, uuid)).toBe(true);
    expect(restored.isFollowing(uuid, utf8)).toBe(true);
    expect(restored.getFollowDistance(bob)).toBe(1);
    expect(restored.getFollowDistance(uuid)).toBe(2);
    expect(restored.getFollowDistance(utf8)).toBe(3);
    expect(restored.getFollowListCreatedAt(bob)).toBe(1100);
    expect(restored.getMuteListCreatedAt(bob)).toBe(1300);
    expect(restored.isMuting(root, carol)).toBe(true);
    expect(restored.getListRevision(root, 3)).toEqual({ id: revId(1000), created_at: 1000, complete: true });
    expect(restored.getListRevision(bob, 3)).toEqual({ id: revId(1100), created_at: 1100, complete: true });
    expect(restored.getListRevision(bob, 10000)).toEqual({ id: revId(1300), created_at: 1300, complete: true });

    // Re-encoding a decoded v5 graph is idempotent.
    const v5Again = await restored.toBinary(undefined, undefined, undefined, undefined, 5);
    expect(v5Again).toEqual(v5);
  });

  it('shrinks real socialGraph.bin in v5 and preserves exact graph semantics', async () => {
    const root = '4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0';
    const v4Bytes = new Uint8Array(fs.readFileSync(path.resolve(import.meta.dirname, '../data/socialGraph.bin')));
    const fromV4 = await SocialGraph.fromBinary(root, v4Bytes);
    const v5Bytes = await fromV4.toBinary(undefined, undefined, undefined, undefined, 5);

    expect(v5Bytes[0]).toBe(5);
    expect(v5Bytes.length).toBeLessThan(v4Bytes.length * 0.85);

    const fromV5 = await fromBinary(root, v5Bytes, true);
    expect(fromV5.size()).toEqual(fromV4.size());
    expect(fromV5.getListRevisions().length).toBe(fromV4.getListRevisions().length);
    for (const u of Array.from(fromV4.userIterator(2)).slice(0, 500)) {
      expect(fromV5.getFollowDistance(u)).toBe(fromV4.getFollowDistance(u));
      expect(fromV5.followerCount(u)).toBe(fromV4.followerCount(u));
      expect(fromV5.isOvermuted(u, 1)).toBe(fromV4.isOvermuted(u, 1));
    }
  });

  it('materializes compact reverse indexes transparently on getInternalData()', async () => {
    const root = key(1);
    const bob = key(2);
    const carol = key(3);
    const graph = new SocialGraph(root);
    graph.handleEvent(event(root, 3, 1000, [bob]), true);
    graph.handleEvent(event(root, 10000, 1000, [carol]), true);

    expect(graph.followerCount(bob)).toBe(1);
    expect(graph.getFollowersByUser(bob)).toEqual(new Set([root]));
    expect(graph.getUserMutedBy(carol)).toEqual(new Set([root]));

    const internal = graph.getInternalData();
    const bobId = internal.ids.get(bob)!;
    const carolId = internal.ids.get(carol)!;
    const rootId = internal.ids.get(root)!;
    expect(internal.followersByUser.get(bobId)).toBeInstanceOf(Set);
    expect(internal.followersByUser.get(bobId)?.has(rootId)).toBe(true);
    expect(internal.userMutedBy.get(carolId)).toBeInstanceOf(Set);
    expect(internal.userMutedBy.get(carolId)?.has(rootId)).toBe(true);
  });

  it('rejects malformed v5 binaries (out-of-range ids, duplicate keys, trailing mobile data)', async () => {
    const root = key(1);
    // Out-of-range owner index
    const badOwner = new Uint8Array([5, 1, ...new Uint8Array(32).fill(1), 0, 0, 1, 1, 10, 0, 0, 0]);
    await expect(fromBinary(root, badOwner)).rejects.toThrow();

    // Duplicate pubkey
    const dupPk = new Uint8Array([5, 2, ...new Uint8Array(32).fill(1), ...new Uint8Array(32).fill(1), 0, 0, 0, 0]);
    await expect(fromBinary(root, dupPk)).rejects.toThrow();

    // Trailing bytes rejected in publicMobile mode
    const graph = new SocialGraph(root);
    graph.handleEvent(event(root, 3, 1000, [key(2)]), true);
    const v5 = await graph.toBinary(undefined, undefined, undefined, undefined, 5);
    const trailing = new Uint8Array(v5.length + 1);
    trailing.set(v5);
    await expect(fromBinary(root, trailing, true)).rejects.toThrow();
  });
});
