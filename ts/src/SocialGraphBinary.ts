import { canonicalizeGraphNodes } from './SocialGraphAliases';
import { canonicalNodeId } from './UniqueIds';
import { SocialGraph } from './SocialGraph';

export const BINARY_FORMAT_VERSION = 4;
const BINARY_FORMAT_VERSION_V2 = 2;
const BINARY_NODE_ID_PUBKEY = 0;
const BINARY_NODE_ID_UUID = 1;
const BINARY_NODE_ID_STRING = 2;

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const pubkeyPattern = /^[0-9a-fA-F]{64}$/;
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder('utf-8', { fatal: true });

function planBudget(
  graph: SocialGraph,
  maxNodes?: number,
  maxEdges?: number,
  maxDistance?: number,
  maxEdgesPerNode?: number
) {
  const usedIds = new Set<number>();
  const followEdgeCount = new Map<number, number>();
  const muteEdgeCount = new Map<number, number>();
  const { followedByUser, mutedByUser, followListCreatedAt, muteListCreatedAt, ids, str } = graph.getInternalData();
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
      for (const [edgesByOwner, counts] of edgeTypes) {
        const targets = edgesByOwner.get(owner);
        if (!targets) continue;
        for (const target of targets) {
          if (maxEdges && edgeCount >= maxEdges) break budget;
          if (maxEdgesPerNode && ownerEdgeCount >= maxEdgesPerNode) continue owners;
          ownerEdgeCount++;

          try {
            str(owner);
            str(target);
          } catch {
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
  for (const owner of new Set([...usedIds, rootId])) {
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

function hexToBytes(hex: string): Uint8Array {
    if (!/^[0-9a-fA-F]+$/.test(hex)) {
        throw new Error(`Invalid hex string: ${hex}`);
    }
    if (hex.length % 2 !== 0) {
        throw new Error(`Hex string must have even length: ${hex}`);
    }
    const bytes = new Uint8Array(hex.length / 2);
    for (let i = 0; i < hex.length; i += 2) {
        bytes[i / 2] = parseInt(hex.substr(i, 2), 16);
    }
    return bytes;
}

const byteHex = Array.from({ length: 256 }, (_, byte) => byte.toString(16).padStart(2, '0'));

function bytesToHex(bytes: Uint8Array): string {
    let hex = '';
    for (const byte of bytes) hex += byteHex[byte];
    return hex;
}

function isCanonicalUuid(value: string): boolean {
    return uuidPattern.test(value);
}

function uuidToBytes(uuid: string): Uint8Array {
    return hexToBytes(uuid.replace(/-/g, ''));
}

function bytesToUuid(bytes: Uint8Array): string {
    const hex = bytesToHex(bytes);
    return [
        hex.slice(0, 8),
        hex.slice(8, 12),
        hex.slice(12, 16),
        hex.slice(16, 20),
        hex.slice(20),
    ].join('-');
}

function writeBinaryNodeId(
    value: string,
    writeVar: (value: number) => void,
    writeBytes: (bytes: Uint8Array) => void,
) {
    if (pubkeyPattern.test(value)) {
        writeVar(BINARY_NODE_ID_PUBKEY);
        writeBytes(hexToBytes(value));
        return;
    }

    if (isCanonicalUuid(value)) {
        writeVar(BINARY_NODE_ID_UUID);
        writeBytes(uuidToBytes(value));
        return;
    }

    const bytes = textEncoder.encode(value);
    writeVar(BINARY_NODE_ID_STRING);
    writeVar(bytes.length);
    writeBytes(bytes);
}

// Variable-length integer decoding
function decodeVarint(bytes: Uint8Array, offset: number): { value: number; bytesRead: number } {
    let value = 0;
    let multiplier = 1;
    for (let bytesRead = 1; bytesRead <= 8; bytesRead++) {
        if (offset + bytesRead > bytes.length) throw new Error('Unexpected end of binary data');
        const byte = bytes[offset + bytesRead - 1];
        value += (byte & 0x7f) * multiplier;
        if (!Number.isSafeInteger(value)) throw new Error('Binary integer exceeds safe range');
        if ((byte & 0x80) === 0) return {value, bytesRead};
        multiplier *= 128;
    }
    throw new Error('Invalid binary integer');
}

// All integers use varint encoding for consistency and simplicity

export async function* toBinaryChunks(graph: SocialGraph, maxNodes?: number, maxEdges?: number, maxDistance?: number, maxEdgesPerNode?: number): AsyncGenerator<Uint8Array> {
    // --- Phase 1: grab internal graph data ---
    const data = graph.getInternalData();

    // If no budget limits are specified, use the original approach
    let usedIds: Set<number>;
    let followEdgeCount: Map<number, number>;
    let muteEdgeCount: Map<number, number>;
    let followOwners: number[];
    let muteOwners: number[];

    if (maxNodes !== undefined || maxEdges !== undefined || maxDistance !== undefined || maxEdgesPerNode !== undefined) {
        // Budget planning using local planBudget function
        const budgetResult = planBudget(graph, maxNodes, maxEdges, maxDistance, maxEdgesPerNode);
        usedIds = budgetResult.usedIds;
        followEdgeCount = budgetResult.followEdgeCount;
        muteEdgeCount = budgetResult.muteEdgeCount;
        followOwners = budgetResult.followOwners;
        muteOwners = budgetResult.muteOwners;
    } else {
        // Original approach: include all data
        usedIds = new Set<number>();
        followEdgeCount = new Map<number, number>();
        muteEdgeCount = new Map<number, number>();

        for (const [user, followedUsers] of data.followedByUser.entries()) {
            usedIds.add(user);
            followEdgeCount.set(user, followedUsers.size);
            for (const followed of followedUsers) {
                usedIds.add(followed);
            }
        }
        // A first empty event can have a timestamp without an adjacency row.
        for (const user of data.followListCreatedAt.keys()) {
            if (!followEdgeCount.has(user)) {
                usedIds.add(user);
                followEdgeCount.set(user, 0);
            }
        }
        for (const [user, mutedUsers] of data.mutedByUser.entries()) {
            usedIds.add(user);
            muteEdgeCount.set(user, mutedUsers.size);
            for (const muted of mutedUsers) {
                usedIds.add(muted);
            }
        }
        for (const user of data.muteListCreatedAt.keys()) {
            if (!muteEdgeCount.has(user)) {
                usedIds.add(user);
                muteEdgeCount.set(user, 0);
            }
        }

        followOwners = Array.from(followEdgeCount.keys());
        muteOwners = Array.from(muteEdgeCount.keys());
    }

    // --- Helper utilities for fast byte writes ---
    const CHUNK_SIZE = 16 * 1024; // 16 KB
    let buf = new Uint8Array(CHUNK_SIZE);
    let pos = 0;

    const out: Uint8Array[] = [];
    
    const flush = () => {
        if (pos === 0) return;
        out.push(buf.subarray(0, pos));
        buf = new Uint8Array(CHUNK_SIZE);
        pos = 0;
    };

    const writeByte = (b: number) => {
        if (pos >= buf.length) flush();
        buf[pos++] = b;
    };

    const writeBytes = (bytes: Uint8Array) => {
        let i = 0;
        while (i < bytes.length) {
            const avail = buf.length - pos;
            if (avail === 0) {
                flush();
                continue;
            }
            const len = Math.min(avail, bytes.length - i);
            buf.set(bytes.subarray(i, i + len), pos);
            pos += len;
            i += len;
        }
    };

    const writeVar = (v: number) => {
        let n = v >>> 0; // ensure unsigned 32-bit
        while (n >= 0x80) {
            writeByte((n & 0x7f) | 0x80);
            n >>>= 7;
        }
        writeByte(n & 0x7f);
    };

    // --- Header ---
    writeVar(BINARY_FORMAT_VERSION);

    // --- uniqueIds block ---
    writeVar(usedIds.size);
    for (const id of usedIds) {
        writeVar(id);
        writeBinaryNodeId(data.ids.str(id), writeVar, writeBytes);
    }

    const writeRevision = (owner: number, kind: number, timestamp: number, complete: boolean) => {
        const revision = graph.getListRevision(data.str(owner), kind);
        if (!revision || revision.created_at !== timestamp) { writeVar(0); return; }
        writeVar(revision.complete && complete ? 2 : 1);
        writeBytes(hexToBytes(revision.id));
    };

    // --- follow lists ---
    writeVar(followOwners.length);
    for (const owner of followOwners) {
        const ts = data.followListCreatedAt.get(owner) ?? 0;
        const limit = followEdgeCount.get(owner)!;
        writeVar(owner);
        writeVar(ts);
        writeRevision(owner, 3, ts, limit === (data.followedByUser.get(owner)?.size ?? 0));
        writeVar(limit);

        let emitted = 0;
        const outs = data.followedByUser.get(owner) || new Set<number>();
        for (const t of outs) {
            if (emitted >= limit) break;
            writeVar(t);
            emitted++;
        }
    }

    // --- mute lists ---
    writeVar(muteOwners.length);
    for (const owner of muteOwners) {
        const ts = data.muteListCreatedAt.get(owner) ?? 0;
        const limit = muteEdgeCount.get(owner)!;
        writeVar(owner);
        writeVar(ts);
        writeRevision(owner, 10000, ts, limit === (data.mutedByUser.get(owner)?.size ?? 0));
        writeVar(limit);

        let emitted = 0;
        const outs = data.mutedByUser.get(owner) || new Set<number>();
        for (const t of outs) {
            if (emitted >= limit) break;
            writeVar(t);
            emitted++;
        }
    }

    // --- Final flush ---
    flush();
    for (const c of out) {
        yield c;
    }
}


export async function toBinary(graph: SocialGraph, maxNodes?: number, maxEdges?: number, maxDistance?: number, maxEdgesPerNode?: number): Promise<Uint8Array> {
    const chunks: Uint8Array[] = [];
    let total = 0;
    
    for await (const c of toBinaryChunks(graph, maxNodes, maxEdges, maxDistance, maxEdgesPerNode)) {
        chunks.push(c);
        total += c.length;
    }
    
    const out = new Uint8Array(total);
    let off = 0;
    for (const c of chunks) { 
        out.set(c, off); 
        off += c.length; 
    }
    return out;
}

export async function fromBinary(root: string, data: Uint8Array): Promise<SocialGraph> {
    let offset = 0;
    
    // Read version
    const version = decodeVarint(data, offset);
    offset += version.bytesRead;
    
    if (version.value !== BINARY_FORMAT_VERSION && version.value !== 3 && version.value !== BINARY_FORMAT_VERSION_V2) {
        throw new Error(`Invalid binary version: ${version.value}`);
    }

    const readBytes = (len: number): Uint8Array => {
        if (offset + len > data.length) {
            throw new Error('Unexpected end of binary data');
        }
        const bytes = data.subarray(offset, offset + len);
        offset += len;
        return bytes;
    };

    // Read unique IDs
    const idsCount = decodeVarint(data, offset);
    offset += idsCount.bytesRead;
    
    const uniqueIds: [string, number][] = [];
    
    for (let i = 0; i < idsCount.value; i++) {
        if (version.value === BINARY_FORMAT_VERSION_V2) {
            const hexBytes = readBytes(32);
            const hexStr = bytesToHex(hexBytes);
            const id = decodeVarint(data, offset);
            offset += id.bytesRead;
            uniqueIds.push([hexStr, id.value]);
            continue;
        }

        const id = decodeVarint(data, offset);
        offset += id.bytesRead;

        const nodeIdType = decodeVarint(data, offset);
        offset += nodeIdType.bytesRead;

        let value: string;
        if (nodeIdType.value === BINARY_NODE_ID_PUBKEY) {
            value = bytesToHex(readBytes(32));
        } else if (nodeIdType.value === BINARY_NODE_ID_UUID) {
            value = bytesToUuid(readBytes(16));
        } else if (nodeIdType.value === BINARY_NODE_ID_STRING) {
            const len = decodeVarint(data, offset);
            offset += len.bytesRead;
            value = textDecoder.decode(readBytes(len.value));
        } else {
            throw new Error(`Invalid binary node id type: ${nodeIdType.value}`);
        }

        if (!value || value.trim() === '') {
            throw new Error('Cannot store empty or whitespace-only strings');
        }
        uniqueIds.push([value, id.value]);
    }
    
    const revisions: [number, number, string, number, boolean][] = [];
    const readRevision = (user: number, kind: number, timestamp: number) => {
        if (version.value < 4) return;
        const flag = readBytes(1)[0];
        if (flag > 2) throw new Error('Invalid list revision flag');
        if (flag) revisions.push([user, kind, bytesToHex(readBytes(32)), timestamp, flag === 2]);
    };

    // Read follow lists
    const followListsCount = decodeVarint(data, offset);
    offset += followListsCount.bytesRead;
    
    const followLists: [number, number[], number][] = [];
    
    for (let i = 0; i < followListsCount.value; i++) {
        const user = decodeVarint(data, offset);
        offset += user.bytesRead;
        
        const timestamp = decodeVarint(data, offset);
        offset += timestamp.bytesRead;
        
        readRevision(user.value, 3, timestamp.value);
        const followedCount = decodeVarint(data, offset);
        offset += followedCount.bytesRead;
        
        const followedUsers: number[] = [];
        
        for (let j = 0; j < followedCount.value; j++) {
            const followedUser = decodeVarint(data, offset);
            offset += followedUser.bytesRead;
            followedUsers.push(followedUser.value);
        }
        
        followLists.push([user.value, followedUsers, timestamp.value]);
    }
    
    // Read mute lists
    const muteListsCount = decodeVarint(data, offset);
    offset += muteListsCount.bytesRead;
    
    const muteLists: [number, number[], number][] = [];
    
    for (let i = 0; i < muteListsCount.value; i++) {
        const user = decodeVarint(data, offset);
        offset += user.bytesRead;
        
        const timestamp = decodeVarint(data, offset);
        offset += timestamp.bytesRead;
        
        readRevision(user.value, 10000, timestamp.value);
        const mutedCount = decodeVarint(data, offset);
        offset += mutedCount.bytesRead;
        
        const mutedUsers: number[] = [];
        
        for (let j = 0; j < mutedCount.value; j++) {
            const mutedUser = decodeVarint(data, offset);
            offset += mutedUser.bytesRead;
            mutedUsers.push(mutedUser.value);
        }
        
        muteLists.push([user.value, mutedUsers, timestamp.value]);
    }
    
    const canonical = canonicalizeGraphNodes(uniqueIds, followLists, muteLists, revisions);
    root = canonicalNodeId(root);

    // Create a new SocialGraph and populate it directly
    const graph = new SocialGraph(root);
    const graphAny = graph as any;
    
    // Clear the UniqueIds mapping and repopulate with serialized data
    graphAny.ids.uniqueIdToStr.clear();
    graphAny.ids.strToUniqueId.clear();
    graphAny.ids.currentUniqueId = 0;
    
    // Populate the UniqueIds mapping
    for (const [value, id] of canonical.uniqueIds) {
        graphAny.ids.uniqueIdToStr.set(id, value);
        graphAny.ids.strToUniqueId.set(value, id);
        graphAny.ids.currentUniqueId = Math.max(graphAny.ids.currentUniqueId, id + 1);
    }
    
    for (const [user, kind, id, created_at, complete] of canonical.revisions) {
        graph.getInternalData().listRevisions.set(`${kind}:${graphAny.ids.str(user)}`, {id, created_at, complete});
    }

    // Ensure the new root is properly mapped in the UniqueIds
    if (!graphAny.ids.strToUniqueId.has(root)) {
        // If the new root wasn't in the original data, add it with a new ID
        const rootId = graphAny.ids.id(root);
        graphAny.root = rootId;
    } else {
        // If the new root was in the original data, use its existing ID
        graphAny.root = graphAny.ids.strToUniqueId.get(root);
    }
    
    graphAny.followDistanceByUser.clear();
    graphAny.usersByFollowDistance.clear();
    graphAny.followedByUser.clear();
    graphAny.followersByUser.clear();
    graphAny.followListCreatedAt.clear();
    graphAny.mutedByUser.clear();
    graphAny.userMutedBy.clear();
    graphAny.muteListCreatedAt.clear();

    // Populate follow lists without deriving distances from serialized order.
    for (const [follower, followedUsers, createdAt] of canonical.followLists) {
        graphAny.followedByUser.set(follower, new Set(followedUsers));
        for (const followedUser of followedUsers) {
            if (!graphAny.followersByUser.has(followedUser)) {
                graphAny.followersByUser.set(followedUser, new Set<number>());
            }
            graphAny.followersByUser.get(followedUser)!.add(follower);
        }
        graphAny.followListCreatedAt.set(follower, createdAt ?? 0);
    }
    
    // Populate mute lists
    for (const [muter, mutedUsers, createdAt] of canonical.muteLists) {
        graphAny.mutedByUser.set(muter, new Set(mutedUsers));
        for (const mutedUser of mutedUsers) {
            if (!graphAny.userMutedBy.has(mutedUser)) {
                graphAny.userMutedBy.set(mutedUser, new Set());
            }
            graphAny.userMutedBy.get(mutedUser)?.add(muter);
        }
        graphAny.muteListCreatedAt.set(muter, createdAt ?? 0);
    }

    await graph.recalculateFollowDistances(1_000, 100_000, () => {});
    
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
    
    // Combine all chunks into a single buffer
    const combined = new Uint8Array(totalLength);
    let offset = 0;
    
    for (const chunk of chunks) {
        combined.set(chunk, offset);
        offset += chunk.length;
    }
    
    return await fromBinary(root, combined);
}
