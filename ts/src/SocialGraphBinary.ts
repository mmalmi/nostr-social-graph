import { canonicalizeGraphNodes } from './SocialGraphAliases.js';
import { canonicalNodeId, UniqueIds } from './UniqueIds.js';
import { SocialGraph, addCompactEdge, CompactEdgeEntry, DistanceTable } from './SocialGraph.js';

export const BINARY_FORMAT_VERSION = 4;
export const BINARY_FORMAT_VERSION_V5 = 5;
const BINARY_FORMAT_VERSION_V2 = 2;
const BINARY_NODE_ID_PUBKEY = 0;
const BINARY_NODE_ID_UUID = 1;
const BINARY_NODE_ID_STRING = 2;
const CHUNK_SIZE = 16 * 1024;

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const pubkeyPattern = /^[0-9a-fA-F]{64}$/;
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder('utf-8', { fatal: true });

const byteHex = Array.from({ length: 256 }, (_, byte) => byte.toString(16).padStart(2, '0'));
const hexNibble = new Int8Array(128).fill(-1);
for (let i = 0; i < 10; i++) hexNibble[48 + i] = i;
for (let i = 0; i < 6; i++) hexNibble[65 + i] = hexNibble[97 + i] = 10 + i;

function hexAt(bytes: Uint8Array, start: number, len: number): string {
  let out = '';
  const end = start + len;
  let i = start;
  for (; i + 4 <= end; i += 4) {
    out += byteHex[bytes[i]] + byteHex[bytes[i + 1]] + byteHex[bytes[i + 2]] + byteHex[bytes[i + 3]];
  }
  for (; i < end; i++) out += byteHex[bytes[i]];
  return out;
}

function uuidAt(bytes: Uint8Array, start: number): string {
  const hex = hexAt(bytes, start, 16);
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function compareBytes(a: Uint8Array, b: Uint8Array): number {
  const minLen = Math.min(a.length, b.length);
  for (let i = 0; i < minLen; i++) {
    const diff = a[i] - b[i];
    if (diff !== 0) return diff;
  }
  return a.length - b.length;
}

interface BinaryPlan {
  usedIds: Set<number>;
  followEdgeCount: Map<number, number>;
  muteEdgeCount: Map<number, number>;
  followOwners: number[];
  muteOwners: number[];
}

function planBudget(
  graph: SocialGraph,
  maxNodes?: number,
  maxEdges?: number,
  maxDistance?: number,
  maxEdgesPerNode?: number
): BinaryPlan {
  const usedIds = new Set<number>();
  const followEdgeCount = new Map<number, number>();
  const muteEdgeCount = new Map<number, number>();
  const graphAny = graph as any;
  const followedByUser: Map<number, Set<number>> = graphAny.followedByUser;
  const mutedByUser: Map<number, Set<number>> = graphAny.mutedByUser;
  const followListCreatedAt: Map<number, number> = graphAny.followListCreatedAt;
  const muteListCreatedAt: Map<number, number> = graphAny.muteListCreatedAt;
  const ids: UniqueIds = graphAny.ids;
  const edgeTypes = [
    [followedByUser, followEdgeCount],
    [mutedByUser, muteEdgeCount],
  ] as const;
  const usersByFollowDistance = graphAny.usersByFollowDistance as Map<number, Set<number>>;
  const followDistances = graphAny.followDistanceByUser as DistanceTable;
  const distances = Array.from(usersByFollowDistance.keys()).sort((a, b) => a - b);
  let edgeCount = 0;

  // Select edges in distance, owner, then follow/mute order. Stop before scanning
  // the rest of the graph once the export budget is exhausted.
  budget: for (const distance of distances) {
    if (maxDistance !== undefined && distance > maxDistance) break;
    owners: for (const owner of usersByFollowDistance.get(distance)!) {
      let ownerEdgeCount = 0;
      const ownerValid = ids.hasId(owner);
      for (const [edgesByOwner, counts] of edgeTypes) {
        const targets = edgesByOwner.get(owner);
        if (!targets) continue;
        for (const target of targets) {
          if (maxEdges && edgeCount >= maxEdges) break budget;
          if (maxEdgesPerNode && ownerEdgeCount >= maxEdgesPerNode) continue owners;
          ownerEdgeCount++;

          if (!ownerValid || !ids.hasId(target)) {
            console.warn(`Skipping edge with invalid ID: owner=${owner}, target=${target}`);
            continue;
          }

          if (maxNodes) {
            const newNodesCount = Number(!usedIds.has(owner)) + Number(!usedIds.has(target));
            if (usedIds.size + newNodesCount > maxNodes) break budget;
          }

          usedIds.add(owner);
          usedIds.add(target);
          edgeCount++;
          counts.set(owner, (counts.get(owner) ?? 0) + 1);
        }
      }
    }
  }

  // Empty lists are facts too. Keep them for retained, in-range owners (and
  // an isolated root), without calling a truncated nonempty list empty.
  const rootId = ids.id(graph.getRoot());
  const candidates = usedIds.has(rootId) ? Array.from(usedIds) : [...usedIds, rootId];
  for (const owner of candidates) {
    const distance = followDistances.get(owner);
    if (distance === undefined || (maxDistance !== undefined && distance > maxDistance)) continue;
    if (!usedIds.has(owner) && maxNodes && usedIds.size >= maxNodes) continue;
    for (const [lists, timestamps, counts] of [
      [followedByUser, followListCreatedAt, followEdgeCount],
      [mutedByUser, muteListCreatedAt, muteEdgeCount],
    ] as const) {
      if (!lists.get(owner)?.size && (lists.has(owner) || timestamps.has(owner))) {
        usedIds.add(owner);
        counts.set(owner, 0);
      }
    }
  }

  return {
    usedIds,
    followEdgeCount,
    muteEdgeCount,
    followOwners: Array.from(followEdgeCount.keys()),
    muteOwners: Array.from(muteEdgeCount.keys()),
  };
}

function planFull(graph: SocialGraph): BinaryPlan {
  const graphAny = graph as any;
  const followedByUser: Map<number, Set<number>> = graphAny.followedByUser;
  const mutedByUser: Map<number, Set<number>> = graphAny.mutedByUser;
  const followListCreatedAt: Map<number, number> = graphAny.followListCreatedAt;
  const muteListCreatedAt: Map<number, number> = graphAny.muteListCreatedAt;
  const usedIds = new Set<number>();
  const followEdgeCount = new Map<number, number>();
  const muteEdgeCount = new Map<number, number>();

  for (const [lists, timestamps, counts] of [
    [followedByUser, followListCreatedAt, followEdgeCount],
    [mutedByUser, muteListCreatedAt, muteEdgeCount],
  ] as const) {
    for (const [user, targets] of lists) {
      usedIds.add(user);
      counts.set(user, targets.size);
      for (const target of targets) usedIds.add(target);
    }
    // A first empty event can have a timestamp without an adjacency row.
    for (const user of timestamps.keys()) {
      if (!counts.has(user)) {
        usedIds.add(user);
        counts.set(user, 0);
      }
    }
  }

  return {
    usedIds,
    followEdgeCount,
    muteEdgeCount,
    followOwners: Array.from(followEdgeCount.keys()),
    muteOwners: Array.from(muteEdgeCount.keys()),
  };
}

function* encodeBinaryChunks(
  graph: SocialGraph,
  maxNodes?: number,
  maxEdges?: number,
  maxDistance?: number,
  maxEdgesPerNode?: number,
  version: 3 | 4 | 5 = BINARY_FORMAT_VERSION
): Generator<Uint8Array> {
  if (version !== 3 && version !== BINARY_FORMAT_VERSION && version !== BINARY_FORMAT_VERSION_V5) {
    throw new Error(`Invalid binary version: ${version}`);
  }
  const graphAny = graph as any;
  const ids: UniqueIds = graphAny.ids;
  const followedByUser: Map<number, Set<number>> = graphAny.followedByUser;
  const mutedByUser: Map<number, Set<number>> = graphAny.mutedByUser;
  const followListCreatedAt: Map<number, number> = graphAny.followListCreatedAt;
  const muteListCreatedAt: Map<number, number> = graphAny.muteListCreatedAt;
  const listRevisions: Map<string, { id: string; created_at: number; complete: boolean }> = graphAny.listRevisions;

  const plan = (maxNodes !== undefined || maxEdges !== undefined || maxDistance !== undefined || maxEdgesPerNode !== undefined)
    ? planBudget(graph, maxNodes, maxEdges, maxDistance, maxEdgesPerNode)
    : planFull(graph);

  let buf = new Uint8Array(CHUNK_SIZE);
  let pos = 0;
  const pending: Uint8Array[] = [];

  const flush = () => {
    if (pos === 0) return;
    pending.push(buf.subarray(0, pos));
    buf = new Uint8Array(CHUNK_SIZE);
    pos = 0;
  };

  const writeByte = (b: number) => {
    if (pos >= CHUNK_SIZE) flush();
    buf[pos++] = b;
  };

  const writeBytes = (bytes: Uint8Array) => {
    let i = 0;
    while (i < bytes.length) {
      const avail = CHUNK_SIZE - pos;
      if (avail === 0) { flush(); continue; }
      const len = Math.min(avail, bytes.length - i);
      buf.set(bytes.subarray(i, i + len), pos);
      pos += len;
      i += len;
    }
  };

  const writeHex = (hex: string, skipDashes = false) => {
    const byteLen = skipDashes ? 16 : (hex.length >> 1);
    if (!skipDashes && pos + byteLen <= CHUNK_SIZE) {
      for (let i = 0; i < hex.length; i += 2) {
        const hi = hexNibble[hex.charCodeAt(i)];
        const lo = hexNibble[hex.charCodeAt(i + 1)];
        if ((hi | lo) < 0) throw new Error(`Invalid hex string: ${hex}`);
        buf[pos++] = (hi << 4) | lo;
      }
      return;
    }
    for (let i = 0; i < hex.length; ) {
      if (skipDashes && hex.charCodeAt(i) === 45) { i++; continue; }
      const hi = hexNibble[hex.charCodeAt(i)];
      const lo = hexNibble[hex.charCodeAt(i + 1)];
      if ((hi | lo) < 0) throw new Error(`Invalid hex string: ${hex}`);
      writeByte((hi << 4) | lo);
      i += 2;
    }
  };

  const writeVar = (v: number) => {
    let n = v >>> 0;
    if (n < 0x80 && pos < CHUNK_SIZE) { buf[pos++] = n; return; }
    while (n >= 0x80) {
      writeByte((n & 0x7f) | 0x80);
      n >>>= 7;
    }
    writeByte(n);
  };

  const writeRevision = (owner: number, kindPrefix: string, ts: number, limit: number, outs: Set<number> | undefined) => {
    const ownerStr = ids.getStr(owner);
    const revision = ownerStr ? listRevisions.get(kindPrefix + ownerStr) : undefined;
    if (!revision || revision.created_at !== ts) {
      writeVar(0);
    } else {
      writeVar(revision.complete && limit === (outs?.size ?? 0) ? 2 : 1);
      writeHex(revision.id);
    }
  };

  if (version === BINARY_FORMAT_VERSION_V5) {
    // Count reference frequency across all exported lists so high-degree hubs
    // receive the lowest dense IDs, shrinking sorted target gap-deltas.
    const freq = new Map<number, number>();
    for (const [counts, lists] of [
      [plan.followEdgeCount, followedByUser],
      [plan.muteEdgeCount, mutedByUser],
    ] as const) {
      for (const [owner, limit] of counts) {
        freq.set(owner, (freq.get(owner) ?? 0) + 1);
        if (limit > 0) {
          const outs = lists.get(owner);
          if (outs) {
            let emitted = 0;
            for (const t of outs) {
              freq.set(t, (freq.get(t) ?? 0) + 1);
              if (++emitted >= limit) break;
            }
          }
        }
      }
    }

    const pubkeys: Array<[number, number, string]> = [];
    const uuids: Array<[number, number, string]> = [];
    const strings: Array<[number, number, Uint8Array]> = [];
    for (const id of plan.usedIds) {
      const value = ids.getStr(id);
      if (!value) throw new Error('pub: invalid id ' + id);
      const f = freq.get(id) ?? 0;
      if (value.length === 64 && pubkeyPattern.test(value)) {
        pubkeys.push([id, f, value]);
      } else if (value.length === 36 && uuidPattern.test(value)) {
        uuids.push([id, f, value]);
      } else {
        strings.push([id, f, textEncoder.encode(value)]);
      }
    }
    pubkeys.sort((a, b) => (b[1] - a[1]) || (a[2] < b[2] ? -1 : a[2] > b[2] ? 1 : 0));
    uuids.sort((a, b) => (b[1] - a[1]) || (a[2] < b[2] ? -1 : a[2] > b[2] ? 1 : 0));
    strings.sort((a, b) => (b[1] - a[1]) || compareBytes(a[2], b[2]));

    const oldToNew = new Map<number, number>();
    let nextId = 0;
    for (let i = 0; i < pubkeys.length; i++) oldToNew.set(pubkeys[i][0], nextId++);
    for (let i = 0; i < uuids.length; i++) oldToNew.set(uuids[i][0], nextId++);
    for (let i = 0; i < strings.length; i++) oldToNew.set(strings[i][0], nextId++);

    writeVar(BINARY_FORMAT_VERSION_V5);
    writeVar(pubkeys.length);
    for (let i = 0; i < pubkeys.length; i++) {
      writeHex(pubkeys[i][2]);
      if (pending.length) { yield* pending; pending.length = 0; }
    }
    writeVar(uuids.length);
    for (let i = 0; i < uuids.length; i++) {
      writeHex(uuids[i][2], true);
      if (pending.length) { yield* pending; pending.length = 0; }
    }
    writeVar(strings.length);
    for (let i = 0; i < strings.length; i++) {
      const encoded = strings[i][2];
      writeVar(encoded.length);
      writeBytes(encoded);
      if (pending.length) { yield* pending; pending.length = 0; }
    }

    const writeListsV5 = (
      owners: number[],
      kind: 3 | 10000,
      lists: Map<number, Set<number>>,
      timestamps: Map<number, number>,
      counts: Map<number, number>,
    ) => {
      const prefix = `${kind}:`;
      const mappedOwners: Array<[number, number]> = owners.map(oldOwner => [oldToNew.get(oldOwner)!, oldOwner]);
      mappedOwners.sort((a, b) => a[0] - b[0]);
      writeVar(mappedOwners.length);
      let prevOwner = 0;
      const mappedTargets: number[] = [];
      for (let i = 0; i < mappedOwners.length; i++) {
        const [newOwner, oldOwner] = mappedOwners[i];
        writeVar(i === 0 ? newOwner : newOwner - prevOwner - 1);
        prevOwner = newOwner;
        const ts = timestamps.get(oldOwner) ?? 0;
        const limit = counts.get(oldOwner)!;
        const outs = lists.get(oldOwner);
        writeVar(ts);
        writeRevision(oldOwner, prefix, ts, limit, outs);
        writeVar(limit);
        if (outs && limit > 0) {
          mappedTargets.length = 0;
          let emitted = 0;
          for (const t of outs) {
            mappedTargets.push(oldToNew.get(t)!);
            if (++emitted >= limit) break;
          }
          mappedTargets.sort((a, b) => a - b);
          let prevTarget = 0;
          for (let j = 0; j < mappedTargets.length; j++) {
            const target = mappedTargets[j];
            writeVar(j === 0 ? target : target - prevTarget - 1);
            prevTarget = target;
          }
        }
      }
    };

    writeListsV5(plan.followOwners, 3, followedByUser, followListCreatedAt, plan.followEdgeCount);
    if (pending.length) { yield* pending; pending.length = 0; }
    writeListsV5(plan.muteOwners, 10000, mutedByUser, muteListCreatedAt, plan.muteEdgeCount);
    flush();
    if (pending.length) yield* pending;
    return;
  }

  // Header & uniqueIds (v3 / v4)
  writeVar(version);
  writeVar(plan.usedIds.size);
  for (const id of plan.usedIds) {
    const value = ids.getStr(id);
    if (!value) throw new Error('pub: invalid id ' + id);
    writeVar(id);
    if (value.length === 64 && pubkeyPattern.test(value)) {
      writeVar(BINARY_NODE_ID_PUBKEY);
      writeHex(value);
    } else if (value.length === 36 && uuidPattern.test(value)) {
      writeVar(BINARY_NODE_ID_UUID);
      writeHex(value, true);
    } else {
      const encoded = textEncoder.encode(value);
      writeVar(BINARY_NODE_ID_STRING);
      writeVar(encoded.length);
      writeBytes(encoded);
    }
    if (pending.length) { yield* pending; pending.length = 0; }
  }

  const writeLists = (
    owners: number[],
    kind: 3 | 10000,
    lists: Map<number, Set<number>>,
    timestamps: Map<number, number>,
    counts: Map<number, number>,
  ) => {
    const prefix = `${kind}:`;
    writeVar(owners.length);
    for (let i = 0; i < owners.length; i++) {
      const owner = owners[i];
      const ts = timestamps.get(owner) ?? 0;
      const limit = counts.get(owner)!;
      const outs = lists.get(owner);
      writeVar(owner);
      writeVar(ts);
      if (version >= 4) {
        writeRevision(owner, prefix, ts, limit, outs);
      }
      writeVar(limit);
      if (outs && limit > 0) {
        let emitted = 0;
        for (const t of outs) {
          writeVar(t);
          if (++emitted >= limit) break;
        }
      }
    }
  };

  writeLists(plan.followOwners, 3, followedByUser, followListCreatedAt, plan.followEdgeCount);
  if (pending.length) { yield* pending; pending.length = 0; }
  writeLists(plan.muteOwners, 10000, mutedByUser, muteListCreatedAt, plan.muteEdgeCount);
  flush();
  if (pending.length) yield* pending;
}

export async function* toBinaryChunks(
  graph: SocialGraph,
  maxNodes?: number,
  maxEdges?: number,
  maxDistance?: number,
  maxEdgesPerNode?: number,
  version: 3 | 4 | 5 = BINARY_FORMAT_VERSION
): AsyncGenerator<Uint8Array> {
  for (const chunk of encodeBinaryChunks(graph, maxNodes, maxEdges, maxDistance, maxEdgesPerNode, version)) {
    yield chunk;
  }
}

export async function toBinary(
  graph: SocialGraph,
  maxNodes?: number,
  maxEdges?: number,
  maxDistance?: number,
  maxEdgesPerNode?: number,
  version: 3 | 4 | 5 = BINARY_FORMAT_VERSION
): Promise<Uint8Array> {
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (const c of encodeBinaryChunks(graph, maxNodes, maxEdges, maxDistance, maxEdgesPerNode, version)) {
    chunks.push(c);
    total += c.length;
  }
  if (chunks.length === 1) return chunks[0];
  const out = new Uint8Array(total);
  let off = 0;
  for (const c of chunks) {
    out.set(c, off);
    off += c.length;
  }
  return out;
}

// Public mobile files accept bounded v4/v5 snapshots. Saved and imported
// graphs retain full backward compatibility across v2, v3, v4, and v5.
export async function fromBinary(root: string, data: Uint8Array, publicMobile = false): Promise<SocialGraph> {
  let offset = 0;
  const len = data.length;

  const readVar = (): number => {
    if (offset >= len) throw new Error('Unexpected end of binary data');
    const b0 = data[offset];
    if (b0 < 0x80) { offset++; return b0; }
    let value = 0;
    let multiplier = 1;
    for (let bytesRead = 1; bytesRead <= 8; bytesRead++) {
      if (offset + bytesRead > len) throw new Error('Unexpected end of binary data');
      const byte = data[offset + bytesRead - 1];
      value += (byte & 0x7f) * multiplier;
      if (!Number.isSafeInteger(value)) throw new Error('Binary integer exceeds safe range');
      if ((byte & 0x80) === 0) { offset += bytesRead; return value; }
      multiplier *= 128;
    }
    throw new Error('Invalid binary integer');
  };

  const skip = (n: number): number => {
    const start = offset;
    const end = start + n;
    if (end > len) throw new Error('Unexpected end of binary data');
    offset = end;
    return start;
  };

  const version = readVar();
  if (publicMobile && version !== BINARY_FORMAT_VERSION && version !== BINARY_FORMAT_VERSION_V5) {
    throw new Error('Invalid mobile graph binary version');
  }
  if (
    version !== BINARY_FORMAT_VERSION_V5 &&
    version !== BINARY_FORMAT_VERSION &&
    version !== 3 &&
    version !== BINARY_FORMAT_VERSION_V2
  ) {
    throw new Error(`Invalid binary version: ${version}`);
  }

  const graph = new SocialGraph(root);
  const graphAny = graph as any;
  const ids: UniqueIds = graphAny.ids;
  ids.clear();
  graphAny.followDistanceByUser.clear();
  graphAny.usersByFollowDistance.clear();
  graphAny.followedByUser.clear();
  graphAny.followersByUser.clear();
  graphAny.followListCreatedAt.clear();
  graphAny.mutedByUser.clear();
  graphAny.userMutedBy.clear();
  graphAny.muteListCreatedAt.clear();

  let edgesLeft = 180_000;

  if (version === BINARY_FORMAT_VERSION_V5) {
    const pubkeyCount = readVar();
    if (publicMobile && pubkeyCount > 30_000) throw new Error('Mobile graph node limit');
    if (pubkeyCount * 32 > len - offset) throw new Error('Unexpected end of binary data');
    for (let i = 0; i < pubkeyCount; i++) {
      const value = hexAt(data, skip(32), 32);
      if (!ids.pushDenseUnique(value)) {
        throw new Error(publicMobile ? 'Invalid mobile graph identities or discovery root' : 'Duplicate identity in v5 binary');
      }
    }

    const uuidCount = readVar();
    if (publicMobile && ids.size + uuidCount > 30_000) throw new Error('Mobile graph node limit');
    if (uuidCount * 16 > len - offset) throw new Error('Unexpected end of binary data');
    for (let i = 0; i < uuidCount; i++) {
      const value = uuidAt(data, skip(16));
      if (!ids.pushDenseUnique(value)) {
        throw new Error(publicMobile ? 'Invalid mobile graph identities or discovery root' : 'Duplicate identity in v5 binary');
      }
    }

    const stringCount = readVar();
    if (publicMobile && ids.size + stringCount > 30_000) throw new Error('Mobile graph node limit');
    for (let i = 0; i < stringCount; i++) {
      const strLen = readVar();
      const start = skip(strLen);
      const decoded = textDecoder.decode(data.subarray(start, start + strLen));
      if (!decoded || decoded.trim() === '') {
        throw new Error('Cannot store empty or whitespace-only strings');
      }
      if (!ids.pushDenseUnique(canonicalNodeId(decoded))) {
        throw new Error(publicMobile ? 'Invalid mobile graph identities or discovery root' : 'Duplicate identity in v5 binary');
      }
    }

    const totalNodes = ids.size;
    if (publicMobile && !ids.has(root)) {
      throw new Error('Invalid mobile graph identities or discovery root');
    }

    const listRevisions: Map<string, { id: string; created_at: number; complete: boolean }> = graphAny.listRevisions;
    const readListsV5 = (
      kind: 3 | 10000,
      forward: Map<number, Set<number>>,
      reverse: Map<number, CompactEdgeEntry>,
      timestamps: Map<number, number>,
    ) => {
      const listsCount = readVar();
      if (listsCount > totalNodes) {
        throw new Error(publicMobile ? 'Mobile graph row limit' : 'Invalid v5 list count');
      }
      const prefix = `${kind}:`;
      let prevOwner = 0;
      for (let i = 0; i < listsCount; i++) {
        const delta = readVar();
        const owner = i === 0 ? delta : prevOwner + 1 + delta;
        if (owner >= totalNodes) {
          throw new Error(publicMobile ? 'Invalid mobile graph list' : 'pub: invalid id ' + owner);
        }
        prevOwner = owner;
        const timestamp = readVar();
        const flag = data[skip(1)];
        if (flag > 2) throw new Error('Invalid list revision flag');
        if (flag) {
          const revId = hexAt(data, skip(32), 32);
          listRevisions.set(prefix + ids.str(owner), { id: revId, created_at: timestamp, complete: flag === 2 });
        }
        const edgeCount = readVar();
        if (edgeCount > totalNodes) {
          throw new Error(publicMobile ? 'Mobile graph edge limit' : 'Invalid v5 edge count');
        }
        if (publicMobile) {
          edgesLeft -= edgeCount;
          if (edgesLeft < 0) throw new Error('Mobile graph edge limit');
        }
        if (edgeCount > 0) {
          const targets = new Set<number>();
          let prevTarget = 0;
          for (let j = 0; j < edgeCount; j++) {
            const tDelta = readVar();
            const target = j === 0 ? tDelta : prevTarget + 1 + tDelta;
            if (target >= totalNodes) {
              throw new Error(publicMobile ? 'Invalid mobile graph list' : 'pub: invalid id ' + target);
            }
            prevTarget = target;
            targets.add(target);
            addCompactEdge(reverse, target, owner, false);
          }
          forward.set(owner, targets);
        }
        timestamps.set(owner, timestamp);
      }
    };

    readListsV5(3, graphAny.followedByUser, graphAny.followersByUser, graphAny.followListCreatedAt);
    readListsV5(10000, graphAny.mutedByUser, graphAny.userMutedBy, graphAny.muteListCreatedAt);
    if (publicMobile && offset !== len) throw new Error('Unexpected trailing mobile graph data');

    root = canonicalNodeId(root);
    graphAny.root = ids.get(root) ?? ids.id(root);
    graph.recalculateFollowDistancesSync();
    return graph;
  }

  const idsCount = readVar();
  if (publicMobile && idsCount > 30_000) throw new Error('Mobile graph node limit');

  let rawUniqueIds: [string, number][] | null = null;

  for (let i = 0; i < idsCount; i++) {
    let value: string;
    let id: number;
    if (version === BINARY_FORMAT_VERSION_V2) {
      value = hexAt(data, skip(32), 32);
      id = readVar();
    } else {
      id = readVar();
      const nodeIdType = readVar();
      if (nodeIdType === BINARY_NODE_ID_PUBKEY) {
        value = hexAt(data, skip(32), 32);
      } else if (nodeIdType === BINARY_NODE_ID_UUID) {
        value = uuidAt(data, skip(16));
      } else if (nodeIdType === BINARY_NODE_ID_STRING) {
        const strLen = readVar();
        const start = skip(strLen);
        const decoded = textDecoder.decode(data.subarray(start, start + strLen));
        if (!decoded || decoded.trim() === '') {
          throw new Error('Cannot store empty or whitespace-only strings');
        }
        value = canonicalNodeId(decoded);
      } else {
        throw new Error(`Invalid binary node id type: ${nodeIdType}`);
      }
    }

    if (rawUniqueIds !== null) {
      rawUniqueIds.push([value, id]);
    } else if (!ids.tryInsertUnique(id, value)) {
      if (publicMobile) throw new Error('Invalid mobile graph identities or discovery root');
      rawUniqueIds = [];
      for (const [eid, evalStr] of ids) rawUniqueIds.push([evalStr, eid]);
      rawUniqueIds.push([value, id]);
    }
  }

  if (publicMobile && !ids.has(root)) {
    throw new Error('Invalid mobile graph identities or discovery root');
  }

  if (!rawUniqueIds) {
    // Fast alias-free path: populate graph maps directly without intermediate arrays or canonicalization pass.
    const listRevisions: Map<string, { id: string; created_at: number; complete: boolean }> = graphAny.listRevisions;

    const readListsDirect = (
      kind: 3 | 10000,
      forward: Map<number, Set<number>>,
      reverse: Map<number, CompactEdgeEntry>,
      timestamps: Map<number, number>,
    ) => {
      const listsCount = readVar();
      if (publicMobile && listsCount > idsCount) throw new Error('Mobile graph row limit');
      const prefix = `${kind}:`;

      for (let i = 0; i < listsCount; i++) {
        const owner = readVar();
        const timestamp = readVar();
        if (version >= 4) {
          const flag = data[skip(1)];
          if (flag > 2) throw new Error('Invalid list revision flag');
          if (flag) {
            const revId = hexAt(data, skip(32), 32);
            const ownerStr = ids.getStr(owner);
            if (!ownerStr) throw new Error('pub: invalid id ' + owner);
            listRevisions.set(prefix + ownerStr, { id: revId, created_at: timestamp, complete: flag === 2 });
          }
        }
        const edgeCount = readVar();
        if (publicMobile) {
          edgesLeft -= edgeCount;
          if (edgesLeft < 0) throw new Error('Mobile graph edge limit');
          if (!ids.hasId(owner) || timestamps.has(owner)) throw new Error('Invalid mobile graph list');
        }

        if (edgeCount > 0) {
          const targets = new Set<number>();
          for (let j = 0; j < edgeCount; j++) {
            const target = readVar();
            if (publicMobile && (!ids.hasId(target) || targets.has(target))) {
              throw new Error('Invalid mobile graph list');
            }
            targets.add(target);
            addCompactEdge(reverse, target, owner, false);
          }
          forward.set(owner, targets);
        } else {
          forward.delete(owner);
        }
        timestamps.set(owner, timestamp);
      }
    };

    readListsDirect(3, graphAny.followedByUser, graphAny.followersByUser, graphAny.followListCreatedAt);
    readListsDirect(10000, graphAny.mutedByUser, graphAny.userMutedBy, graphAny.muteListCreatedAt);
    if (publicMobile && offset !== len) throw new Error('Unexpected trailing mobile graph data');

    root = canonicalNodeId(root);
    graphAny.root = ids.get(root) ?? ids.id(root);
    graph.recalculateFollowDistancesSync();
    return graph;
  }

  // Legacy aliased snapshot path: decode lists into tuples and repair aliases identically.
  const revisions: [number, number, string, number, boolean][] = [];
  const readListsRaw = (kind: 3 | 10000): [number, number[], number][] => {
    const listsCount = readVar();
    const lists: [number, number[], number][] = [];
    for (let i = 0; i < listsCount; i++) {
      const owner = readVar();
      const timestamp = readVar();
      if (version >= 4) {
        const flag = data[skip(1)];
        if (flag > 2) throw new Error('Invalid list revision flag');
        if (flag) revisions.push([owner, kind, hexAt(data, skip(32), 32), timestamp, flag === 2]);
      }
      const edgeCount = readVar();
      const targets = new Array<number>(edgeCount);
      for (let j = 0; j < edgeCount; j++) targets[j] = readVar();
      lists.push([owner, targets, timestamp]);
    }
    return lists;
  };

  const followLists = readListsRaw(3);
  const muteLists = readListsRaw(10000);
  const canonical = canonicalizeGraphNodes(rawUniqueIds, followLists, muteLists, revisions);
  root = canonicalNodeId(root);

  ids.clear();
  for (const [value, id] of canonical.uniqueIds) {
    ids.tryInsertUnique(id, value);
  }

  for (const [user, kind, id, created_at, complete] of canonical.revisions) {
    graphAny.listRevisions.set(`${kind}:${ids.str(user)}`, { id, created_at, complete });
  }

  graphAny.root = ids.get(root) ?? ids.id(root);

  const populate = (
    lists: [number, number[], number][],
    forward: Map<number, Set<number>>,
    reverse: Map<number, CompactEdgeEntry>,
    timestamps: Map<number, number>,
  ) => {
    for (const [owner, targets, createdAt] of lists) {
      if (targets.length > 0) {
        const set = new Set(targets);
        forward.set(owner, set);
        for (const target of set) {
          addCompactEdge(reverse, target, owner, false);
        }
      }
      timestamps.set(owner, createdAt ?? 0);
    }
  };

  populate(canonical.followLists, graphAny.followedByUser, graphAny.followersByUser, graphAny.followListCreatedAt);
  populate(canonical.muteLists, graphAny.mutedByUser, graphAny.userMutedBy, graphAny.muteListCreatedAt);
  graph.recalculateFollowDistancesSync();
  return graph;
}

export async function fromBinaryStream(root: string, stream: ReadableStream<Uint8Array>): Promise<SocialGraph> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let totalLength = 0;

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      totalLength += value.length;
    }
  } finally {
    reader.releaseLock();
  }

  const combined = new Uint8Array(totalLength);
  let offset = 0;
  for (const chunk of chunks) {
    combined.set(chunk, offset);
    offset += chunk.length;
  }

  return await fromBinary(root, combined);
}
