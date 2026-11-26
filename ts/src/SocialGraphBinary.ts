import { canonicalizeGraphNodes } from './SocialGraphAliases.js';
import { canonicalNodeId } from './UniqueIds.js';
import { SocialGraph } from './SocialGraph.js';

export const BINARY_FORMAT_VERSION = 4;
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
  const { followedByUser, mutedByUser, followListCreatedAt, muteListCreatedAt, ids } = graph.getInternalData();
  const idMap = (ids as any).uniqueIdToStr as Map<number, string>;
  const edgeTypes = [
    [followedByUser, followEdgeCount],
    [mutedByUser, muteEdgeCount],
  ] as const;
  const usersByFollowDistance = (graph as any).usersByFollowDistance as Map<number, Set<number>>;
  const followDistances = (graph as any).followDistanceByUser as Map<number, number>;
  const distances = Array.from(usersByFollowDistance.keys()).sort((a, b) => a - b);
  let edgeCount = 0;

  // Select edges in distance, owner, then follow/mute order. Stop before scanning
  // the rest of the graph once the export budget is exhausted.
  budget: for (const distance of distances) {
    if (maxDistance !== undefined && distance > maxDistance) break;
    owners: for (const owner of usersByFollowDistance.get(distance)!) {
      let ownerEdgeCount = 0;
      const ownerValid = idMap.has(owner);
      for (const [edgesByOwner, counts] of edgeTypes) {
        const targets = edgesByOwner.get(owner);
        if (!targets) continue;
        for (const target of targets) {
          if (maxEdges && edgeCount >= maxEdges) break budget;
          if (maxEdgesPerNode && ownerEdgeCount >= maxEdgesPerNode) continue owners;
          ownerEdgeCount++;

          if (!ownerValid || !idMap.has(target)) {
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
  const { followedByUser, mutedByUser, followListCreatedAt, muteListCreatedAt } = graph.getInternalData();
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
  version: 3 | 4 = BINARY_FORMAT_VERSION
): Generator<Uint8Array> {
  const data = graph.getInternalData();
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

  const idToStr = (data.ids as any).uniqueIdToStr as Map<number, string>;

  // Header & uniqueIds
  writeVar(version);
  writeVar(plan.usedIds.size);
  for (const id of plan.usedIds) {
    const value = idToStr.get(id);
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
        const ownerStr = idToStr.get(owner);
        const revision = ownerStr ? data.listRevisions.get(prefix + ownerStr) : undefined;
        if (!revision || revision.created_at !== ts) {
          writeVar(0);
        } else {
          writeVar(revision.complete && limit === (outs?.size ?? 0) ? 2 : 1);
          writeHex(revision.id);
        }
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

  writeLists(plan.followOwners, 3, data.followedByUser, data.followListCreatedAt, plan.followEdgeCount);
  if (pending.length) { yield* pending; pending.length = 0; }
  writeLists(plan.muteOwners, 10000, data.mutedByUser, data.muteListCreatedAt, plan.muteEdgeCount);
  flush();
  if (pending.length) yield* pending;
}

export async function* toBinaryChunks(
  graph: SocialGraph,
  maxNodes?: number,
  maxEdges?: number,
  maxDistance?: number,
  maxEdgesPerNode?: number,
  version: 3 | 4 = BINARY_FORMAT_VERSION
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
  version: 3 | 4 = BINARY_FORMAT_VERSION
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

// Public mobile files have a fixed v4 schema and bounded rows. Saved and
// imported graphs retain the compatibility rules for older binary versions.
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
  if (publicMobile && version !== BINARY_FORMAT_VERSION) throw new Error('Invalid mobile graph binary version');
  if (version !== BINARY_FORMAT_VERSION && version !== 3 && version !== BINARY_FORMAT_VERSION_V2) {
    throw new Error(`Invalid binary version: ${version}`);
  }

  const idsCount = readVar();
  if (publicMobile && idsCount > 30_000) throw new Error('Mobile graph node limit');

  const graph = new SocialGraph(root);
  const graphAny = graph as any;
  const uniqueIdToStr: Map<number, string> = graphAny.ids.uniqueIdToStr;
  const strToUniqueId: Map<string, number> = graphAny.ids.strToUniqueId;
  uniqueIdToStr.clear();
  strToUniqueId.clear();
  let nextUniqueId = 0;

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
    } else if (uniqueIdToStr.has(id) || strToUniqueId.has(value)) {
      if (publicMobile) throw new Error('Invalid mobile graph identities or discovery root');
      rawUniqueIds = [];
      for (const [eid, evalStr] of uniqueIdToStr) rawUniqueIds.push([evalStr, eid]);
      rawUniqueIds.push([value, id]);
    } else {
      uniqueIdToStr.set(id, value);
      strToUniqueId.set(value, id);
      if (id + 1 > nextUniqueId) nextUniqueId = id + 1;
    }
  }

  if (publicMobile && !strToUniqueId.has(root)) {
    throw new Error('Invalid mobile graph identities or discovery root');
  }

  graphAny.followDistanceByUser.clear();
  graphAny.usersByFollowDistance.clear();
  graphAny.followedByUser.clear();
  graphAny.followersByUser.clear();
  graphAny.followListCreatedAt.clear();
  graphAny.mutedByUser.clear();
  graphAny.userMutedBy.clear();
  graphAny.muteListCreatedAt.clear();

  let edgesLeft = 180_000;

  if (!rawUniqueIds) {
    // Fast alias-free path: populate graph maps directly without intermediate arrays or canonicalization pass.
    graphAny.ids.currentUniqueId = nextUniqueId;
    const listRevisions: Map<string, { id: string; created_at: number; complete: boolean }> = graphAny.listRevisions;

    const readListsDirect = (
      kind: 3 | 10000,
      forward: Map<number, Set<number>>,
      reverse: Map<number, Set<number>>,
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
            const ownerStr = uniqueIdToStr.get(owner);
            if (!ownerStr) throw new Error('pub: invalid id ' + owner);
            listRevisions.set(prefix + ownerStr, { id: revId, created_at: timestamp, complete: flag === 2 });
          }
        }
        const edgeCount = readVar();
        if (publicMobile) {
          edgesLeft -= edgeCount;
          if (edgesLeft < 0) throw new Error('Mobile graph edge limit');
          if (!uniqueIdToStr.has(owner) || forward.has(owner)) throw new Error('Invalid mobile graph list');
        }

        const targets = new Set<number>();
        for (let j = 0; j < edgeCount; j++) {
          const target = readVar();
          if (publicMobile && (!uniqueIdToStr.has(target) || targets.has(target))) {
            throw new Error('Invalid mobile graph list');
          }
          targets.add(target);
          let rev = reverse.get(target);
          if (!rev) reverse.set(target, rev = new Set<number>());
          rev.add(owner);
        }
        forward.set(owner, targets);
        timestamps.set(owner, timestamp);
      }
    };

    readListsDirect(3, graphAny.followedByUser, graphAny.followersByUser, graphAny.followListCreatedAt);
    readListsDirect(10000, graphAny.mutedByUser, graphAny.userMutedBy, graphAny.muteListCreatedAt);
    if (publicMobile && offset !== len) throw new Error('Unexpected trailing mobile graph data');

    root = canonicalNodeId(root);
    graphAny.root = strToUniqueId.get(root) ?? graphAny.ids.id(root);
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

  uniqueIdToStr.clear();
  strToUniqueId.clear();
  nextUniqueId = 0;
  for (const [value, id] of canonical.uniqueIds) {
    uniqueIdToStr.set(id, value);
    strToUniqueId.set(value, id);
    if (id + 1 > nextUniqueId) nextUniqueId = id + 1;
  }
  graphAny.ids.currentUniqueId = nextUniqueId;

  for (const [user, kind, id, created_at, complete] of canonical.revisions) {
    graphAny.listRevisions.set(`${kind}:${graphAny.ids.str(user)}`, { id, created_at, complete });
  }

  graphAny.root = strToUniqueId.get(root) ?? graphAny.ids.id(root);

  const populate = (
    lists: [number, number[], number][],
    forward: Map<number, Set<number>>,
    reverse: Map<number, Set<number>>,
    timestamps: Map<number, number>,
  ) => {
    for (const [owner, targets, createdAt] of lists) {
      forward.set(owner, new Set(targets));
      for (const target of targets) {
        let rev = reverse.get(target);
        if (!rev) reverse.set(target, rev = new Set<number>());
        rev.add(owner);
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
