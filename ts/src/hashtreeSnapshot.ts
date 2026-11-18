import { BLOB_MAX_BYTES, HashTree, LinkType, sha256, toHex, type CID, type Store } from '@hashtree/core';
import { parseHashtreeRootEvent, HASHTREE_ROOT_KIND } from '@hashtree/nostr';
import { nip19, verifyEvent, type Event } from 'nostr-tools';

/** Signing identity of the named directory, independent of the scoring root in its manifest. */
export const DEFAULT_GRAPH_SNAPSHOT = {
  publisher: 'npub1yscmnnadrmvape0mvgzfx9uyl267klnhzy8znsr6ysl8wtxngpqsnx6dm0',
  treeName: 'social-graph',
  relays: ['wss://temp.iris.to', 'wss://relay.primal.net'],
  blossomServers: ['https://cdn.iris.to'],
} as const;

export interface GraphSnapshotOptions {
  publisher?: string;
  treeName?: string;
  relays?: readonly string[];
  blossomServers?: readonly string[];
  signal?: AbortSignal;
}

export interface SnapshotFile {
  size: number;
  sha256: string;
}

const fileNames = ['social-graph.bin', 'social-graph-v4.bin', 'mobile.bin', 'allowlist.txt', 'profile-data.json', 'profile-index.json'] as const;
export type GraphSnapshotFile = typeof fileNames[number] | `mobile-${string}.bin`;
export interface GraphSnapshotManifest {
  version: 1;
  root: string;
  graph_completed_at: number;
  graph_since: number;
  generated_at: number;
  files: Record<typeof fileNames[number], SnapshotFile>;
  mobile_roots?: Record<string, SnapshotFile & {file: string}>;
}

export interface GraphSnapshot {
  publisher: string;
  treeName: string;
  /** Signed name event and immutable directory hash for this generation. */
  eventId: string;
  directoryHash: string;
  manifest: GraphSnapshotManifest;
  readFile(name: GraphSnapshotFile, options?: {signal?: AbortSignal; maxBytes?: number}): Promise<Uint8Array>;
}

const HEX = /^[0-9a-f]{64}$/;
const MANIFEST_LIMIT = 64 * 1024;
export const MOBILE_GRAPH_MAX_BYTES = 4 * 1024 * 1024;
export const PUBLIC_MOBILE_MIN_USERS = 10_000;

/** Apply a public discovery floor only to named snapshots, never to retained personal graphs. */
export function checkMobileGraphCoverage(users: number, publisher: string, treeName: string, minUsers?: number) {
  const standard = publisher === DEFAULT_GRAPH_SNAPSHOT.publisher && treeName === DEFAULT_GRAPH_SNAPSHOT.treeName;
  if (minUsers !== undefined && (!Number.isSafeInteger(minUsers) || minUsers < 0 || minUsers > 30_000)) {
    throw new Error('Invalid mobile graph coverage minimum');
  }
  const minimum = Math.max(standard ? PUBLIC_MOBILE_MIN_USERS : 0, minUsers ?? 0);
  if (users < minimum) throw new Error('Insufficient mobile graph discovery coverage');
}


function deadline(signal: AbortSignal | undefined, milliseconds: number) {
  const timeout = AbortSignal.timeout(milliseconds);
  return signal ? AbortSignal.any([signal, timeout]) : timeout;
}

async function resolveDirectory(publisher: string, treeName: string, relays: string[], signal: AbortSignal) {
  const decoded = nip19.decode(publisher);
  if (decoded.type !== 'npub' || !treeName || treeName.includes('/') || !relays.length) throw new Error('Invalid named graph snapshot');
  const author = decoded.data;
  const filter = {kinds: [HASHTREE_ROOT_KIND], authors: [author], '#d': [treeName], '#l': ['hashtree'], limit: 1};
  const done = new AbortController();
  const active = AbortSignal.any([signal, done.signal]);
  const subscription = 'graph-name';
  // Own the sockets so that resolving one relay closes even a second relay
  // whose handshake is still in progress. Bootstrap never waits for all EOSEs.
  const query = (url: string) => new Promise<{cid: CID; eventId: string}>((resolve, reject) => {
    active.throwIfAborted();
    const socket = new WebSocket(url);
    const abort = () => { socket.close(); reject(active.reason); };
    active.addEventListener('abort', abort, {once: true});
    socket.onopen = () => socket.send(JSON.stringify(['REQ', subscription, filter]));
    socket.onerror = () => reject(new Error('Graph snapshot relay unavailable'));
    socket.onclose = () => { active.removeEventListener('abort', abort); reject(new Error('Graph snapshot relay closed')); };
    socket.onmessage = ({data}) => {
      if (typeof data !== 'string' || data.length > 65536) return;
      try {
        const [type, id, event] = JSON.parse(data) as [string, string, Event];
        if (id !== subscription) return;
        if (type === 'EOSE' || type === 'CLOSED') {
          socket.close();
          reject(new Error('Signed graph snapshot unavailable'));
          return;
        }
        if (type !== 'EVENT' || event.pubkey !== author || event.kind !== HASHTREE_ROOT_KIND ||
            event.created_at > Date.now() / 1000 + 300 || !verifyEvent(event)) return;
        const root = parseHashtreeRootEvent(event);
        if (root?.treeName === treeName && root.visibility === 'public' &&
            root.labels.includes('hashtree') && root.rootCid.key?.length === 32) {
          resolve({cid: root.rootCid, eventId: event.id});
        }
      } catch { /* A malformed frame cannot resolve a signed name. */ }
    };
  });
  try {
    return await new Promise<{cid: CID; eventId: string}>((resolve, reject) => {
      const distinct = [...new Set(relays)];
      let remaining = distinct.length;
      for (const relay of distinct) query(relay).then(resolve, error => {
        if (--remaining === 0) reject(error);
      });
    });
  } catch {
    if (signal.aborted) throw signal.reason;
    throw new Error('Signed graph snapshot unavailable');
  } finally {
    done.abort();
  }
}

/** Bound even a chunked response from a server which ignores its content address. */
async function responseBytes(response: Response, limit: number): Promise<Uint8Array> {
  const reader = response.body?.getReader();
  if (!reader) throw new Error('Graph block response has no body');
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    if (Number(response.headers.get('content-length')) > limit) throw new Error('Graph block exceeds byte limit');
    for (;;) {
      const {value, done} = await reader.read();
      if (done) break;
      size += value.length;
      if (size > limit) throw new Error('Graph block exceeds byte limit');
      chunks.push(value);
    }
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  return bytes;
}

function publicTree(servers: readonly string[], signal: AbortSignal): HashTree {
  // Each read passes the caller's AbortSignal to the transport, including body streaming.
  const store: Store = {
    async get(hash) {
      const address = toHex(hash);
      for (const server of servers) {
        signal.throwIfAborted();
        try {
          const url = server.replace(/\/+$/, '') + '/' + address + '.bin';
          const response = await fetch(url, {signal, credentials: 'omit', redirect: 'error'});
          if (!response.ok) { await response.body?.cancel(); continue; }
          const block = await responseBytes(response, BLOB_MAX_BYTES);
          if (toHex(await sha256(block)) === address) return block;
        } catch (error) {
          if (signal.aborted) throw error;
        }
      }
      throw new Error('Verified graph snapshot block unavailable');
    },
    async put() { throw new Error('Read-only graph snapshot'); },
    async delete() { throw new Error('Read-only graph snapshot'); },
    async has(hash) { return (await this.get(hash)) !== null; },
  };
  return new HashTree({store});
}

function validFile(file: SnapshotFile | undefined) {
  return !!file && Number.isSafeInteger(file.size) && file.size >= 0 && HEX.test(file.sha256);
}

function parseManifest(bytes: Uint8Array): GraphSnapshotManifest {
  const manifest = JSON.parse(new TextDecoder('utf-8', {fatal: true}).decode(bytes)) as GraphSnapshotManifest;
  if (!manifest || manifest.version !== 1 || !HEX.test(manifest.root) ||
      ![manifest.graph_since, manifest.graph_completed_at, manifest.generated_at].every(t => Number.isSafeInteger(t) && t >= 0) ||
      !manifest.graph_completed_at || manifest.graph_since > manifest.graph_completed_at ||
      manifest.graph_completed_at > manifest.generated_at || manifest.generated_at > Date.now() / 1000 + 300 ||
      !fileNames.every(name => validFile(manifest.files?.[name])) ||
      manifest.files['mobile.bin'].size > MOBILE_GRAPH_MAX_BYTES) throw new Error('Invalid graph snapshot manifest');
  for (const [root, entry] of Object.entries(manifest.mobile_roots ?? {})) {
    if (!HEX.test(root) || !validFile(entry) || entry.size > MOBILE_GRAPH_MAX_BYTES || entry.file !== `mobile-${root}.bin`) {
      throw new Error('Invalid graph snapshot mobile projection');
    }
  }
  // The public preview and later reads use the same authenticated metadata.
  for (const file of Object.values(manifest.files)) Object.freeze(file);
  Object.freeze(manifest.files);
  if (manifest.mobile_roots) {
    for (const file of Object.values(manifest.mobile_roots)) Object.freeze(file);
    Object.freeze(manifest.mobile_roots);
  }
  return Object.freeze(manifest);
}

/** Choose only the requested discovery region from the signed manifest. */
export function mobileGraphFile(manifest: GraphSnapshotManifest, root: string): GraphSnapshotFile {
  if (!HEX.test(root)) throw new Error('Invalid graph discovery root');
  if (root === manifest.root) return 'mobile.bin';
  if (manifest.mobile_roots?.[root]?.file === `mobile-${root}.bin`) return `mobile-${root}.bin`;
  throw new Error('Graph snapshot has no mobile projection for this root');
}

/** Resolve once. All subsequent files are read through the same immutable CID. */
export async function openGraphSnapshot(options: GraphSnapshotOptions = {}): Promise<GraphSnapshot> {
  const publisher = options.publisher ?? DEFAULT_GRAPH_SNAPSHOT.publisher;
  const name = options.treeName ?? DEFAULT_GRAPH_SNAPSHOT.treeName;
  const servers = [...(options.blossomServers ?? DEFAULT_GRAPH_SNAPSHOT.blossomServers)];
  if (!servers.length) throw new Error('Graph snapshot requires a content server');
  const signal = deadline(options.signal, 8000);
  const signed = await resolveDirectory(publisher, name, [...(options.relays ?? DEFAULT_GRAPH_SNAPSHOT.relays)], signal);
  const tree = publicTree(servers, signal);
  const entries = await tree.listDirectory(signed.cid, signal);
  const directory = new Map(entries.map(entry => [entry.name, entry]));
  const manifestEntry = directory.get('manifest.json');
  if (directory.size !== entries.length || !manifestEntry || manifestEntry.type === LinkType.Dir ||
      manifestEntry.size > MANIFEST_LIMIT) throw new Error('Missing or oversized graph snapshot manifest');
  const raw = await tree.readFile(manifestEntry.cid, {maxBytes: MANIFEST_LIMIT});
  if (!raw || raw.length !== manifestEntry.size) throw new Error('Invalid graph snapshot manifest size');
  const manifest = parseManifest(raw);
  const files = new Map<string, SnapshotFile>();
  for (const name of fileNames) files.set(name, {...manifest.files[name]});
  for (const entry of Object.values(manifest.mobile_roots ?? {})) files.set(entry.file, {...entry});
  return {
    publisher, treeName: name, manifest, eventId: signed.eventId, directoryHash: toHex(signed.cid.hash),
    async readFile(name, {maxBytes, signal: readSignal} = {}) {
      const file = files.get(name);
      const entry = directory.get(name);
      if (!file || !entry || entry.type === LinkType.Dir || entry.size !== file.size ||
          (maxBytes !== undefined && (!Number.isSafeInteger(maxBytes) || maxBytes < 0 || file.size > maxBytes))) {
        throw new Error('Missing graph snapshot file or size exceeds limit');
      }
      const combined = options.signal && readSignal ? AbortSignal.any([options.signal, readSignal]) : readSignal ?? options.signal;
      const active = deadline(combined, 120000);
      active.throwIfAborted();
      const data = await publicTree(servers, active).readFile(entry.cid, {maxBytes: file.size});
      active.throwIfAborted();
      if (!data || data.length !== file.size || toHex(await sha256(data)) !== file.sha256) throw new Error('Graph snapshot file digest mismatch');
      return data;
    },
  };
}
