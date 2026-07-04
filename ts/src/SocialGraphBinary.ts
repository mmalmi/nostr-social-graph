import { SocialGraph } from './SocialGraph';

export const BINARY_FORMAT_VERSION = 3;
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
  const validEdges: Array<{owner: number, target: number, isFollow: boolean}> = [];

  const { followedByUser, mutedByUser } = graph.getInternalData();
  const usersByFollowDistance = (graph as any).usersByFollowDistance as Map<number, Set<number>>;

  const allDistances = Array.from(usersByFollowDistance.keys()).sort((a: number, b: number) => a - b);
  // Filter distances by maxDistance if specified
  const distances = maxDistance !== undefined 
    ? allDistances.filter((d: number) => d <= maxDistance)
    : allDistances;

  // Collect all potential edges first, respecting distance and per-node limits
  const potentialEdges: Array<{owner: number, target: number, isFollow: boolean, distance: number}> = [];
  
  for (const d of distances) {
    const users = usersByFollowDistance.get(d);
    if (!users) continue;
    
    for (const owner of users) {
      let ownerEdgeCount = 0;
      
      // Collect follow edges for this owner
      const outsF = followedByUser.get(owner);
      if (outsF) {
        for (const target of outsF) {
          if (!maxEdgesPerNode || ownerEdgeCount < maxEdgesPerNode) {
            potentialEdges.push({owner, target, isFollow: true, distance: d});
            ownerEdgeCount++;
          }
        }
      }
      
      // Collect mute edges for this owner
      const outsM = mutedByUser.get(owner);
      if (outsM) {
        for (const target of outsM) {
          if (!maxEdgesPerNode || ownerEdgeCount < maxEdgesPerNode) {
            potentialEdges.push({owner, target, isFollow: false, distance: d});
            ownerEdgeCount++;
          }
        }
      }
    }
  }

  // Now process edges in distance order, checking both node and edge limits
  let edgeCount = 0;
  const { str } = graph.getInternalData();
  
  for (const edge of potentialEdges) {
    // Check edge limit
    if (maxEdges && edgeCount >= maxEdges) break;
    
    // Validate that both owner and target actually exist in the UniqueIds mapping
    try {
      str(edge.owner);
      str(edge.target);
    } catch (error) {
      // Skip edges that reference non-existent IDs
      console.warn(`Skipping edge with invalid ID: owner=${edge.owner}, target=${edge.target}`);
      continue;
    }
    
    // Check if we can add both nodes without exceeding maxNodes
    if (maxNodes) {
      const ownerIsNew = !usedIds.has(edge.owner);
      const targetIsNew = !usedIds.has(edge.target);
      const newNodesCount = (ownerIsNew ? 1 : 0) + (targetIsNew ? 1 : 0);
      
      if (usedIds.size + newNodesCount > maxNodes) {
        // Adding this edge would exceed the node limit
        break; // Stop processing once we hit the node limit
      }
    }
    
    // Add the edge
    validEdges.push(edge);
    usedIds.add(edge.owner);
    usedIds.add(edge.target);
    edgeCount++;
    
    // Update edge counts per owner
    const map = edge.isFollow ? followEdgeCount : muteEdgeCount;
    map.set(edge.owner, (map.get(edge.owner) ?? 0) + 1);
  }

  // owners we actually kept
  const followOwners = Array.from(followEdgeCount.keys());
  const muteOwners = Array.from(muteEdgeCount.keys());

  return {
    usedIds,
    followEdgeCount,
    muteEdgeCount,
    followOwners,
    muteOwners,
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

// Convert Uint8Array to hex string
function bytesToHex(bytes: Uint8Array): string {
    return Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
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
    let shift = 0;
    let bytesRead = 0;
    
    for (let i = offset; i < bytes.length; i++) {
        const byte = bytes[i];
        value |= (byte & 0x7F) << shift;
        bytesRead++;
        
        if ((byte & 0x80) === 0) {
            break;
        }
        shift += 7;
    }
    
    return { value, bytesRead };
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
        for (const [user, mutedUsers] of data.mutedByUser.entries()) {
            usedIds.add(user);
            muteEdgeCount.set(user, mutedUsers.size);
            for (const muted of mutedUsers) {
                usedIds.add(muted);
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

    // --- follow lists ---
    writeVar(followOwners.length);
    for (const owner of followOwners) {
        const ts = data.followListCreatedAt.get(owner) ?? 0;
        const limit = followEdgeCount.get(owner)!;
        writeVar(owner);
        writeVar(ts);
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
    
    if (version.value !== BINARY_FORMAT_VERSION && version.value !== BINARY_FORMAT_VERSION_V2) {
        throw new Error(`Invalid binary version: ${version.value}`);
    }

    const readBytes = (len: number): Uint8Array => {
        if (offset + len > data.length) {
            throw new Error('Unexpected end of binary data');
        }
        const bytes = data.slice(offset, offset + len);
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
    
    // Read follow lists
    const followListsCount = decodeVarint(data, offset);
    offset += followListsCount.bytesRead;
    
    const followLists: [number, number[], number][] = [];
    
    for (let i = 0; i < followListsCount.value; i++) {
        const user = decodeVarint(data, offset);
        offset += user.bytesRead;
        
        const timestamp = decodeVarint(data, offset);
        offset += timestamp.bytesRead;
        
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
    
    // Create a new SocialGraph and populate it directly
    const graph = new SocialGraph(root);
    const graphAny = graph as any;
    
    // Clear the UniqueIds mapping and repopulate with serialized data
    graphAny.ids.uniqueIdToStr.clear();
    graphAny.ids.strToUniqueId.clear();
    graphAny.ids.currentUniqueId = 0;
    
    // Populate the UniqueIds mapping
    for (const [value, id] of uniqueIds) {
        graphAny.ids.uniqueIdToStr.set(id, value);
        graphAny.ids.strToUniqueId.set(value, id);
        graphAny.ids.currentUniqueId = Math.max(graphAny.ids.currentUniqueId, id + 1);
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
    for (const [follower, followedUsers, createdAt] of followLists) {
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
    for (const [muter, mutedUsers, createdAt] of muteLists) {
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
