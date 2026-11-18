import { sha256, toHex } from '@hashtree/core';
import { nip19 } from 'nostr-tools';
import { SocialGraph } from './SocialGraph.js';
import { fromBinary } from './SocialGraphBinary.js';
import { checkMobileGraphCoverage, MOBILE_GRAPH_MAX_BYTES, mobileGraphFile, openGraphSnapshot, type GraphSnapshot, type GraphSnapshotOptions } from './hashtreeSnapshot.js';

interface RevisionCounts { ids: number; complete: number }
interface MobileGraphCounts {
  users: number;
  follows: number;
  mutes: number;
  directFollows: number;
  secondDegree: number;
  revisionCounts: { follows: RevisionCounts; mutes: RevisionCounts };
}

export interface MobileGraphReceipt extends MobileGraphCounts {
  version: 2;
  root: string;
  bytes: number;
  sha256: string;
  source: {
    url: string;
    publisher: string;
    treeName: string;
    eventId: string;
    directoryHash: string;
    file: string;
    graphSince: number;
    graphCompletedAt: number;
    generatedAt: number;
    formatVersion: 4;
    bytes: number;
    sha256: string;
  };
}

const hex = /^[0-9a-f]{64}$/;

function checkBytes(root: string, bytes: Uint8Array) {
  if (!hex.test(root) || bytes[0] !== 4 || bytes.length > MOBILE_GRAPH_MAX_BYTES) {
    throw new Error('Invalid mobile graph root, binary version or byte size');
  }
}

async function inspect(root: string, bytes: Uint8Array) {
  checkBytes(root, bytes);
  const graph = await fromBinary(root, bytes, true);
  const {mutedByUser, str} = graph.getInternalData();
  const {users, follows, mutes, sizeByDistance} = graph.size();
  const directFollows = graph.getFollowedByUser(root).size;
  if (!graph.getListRevision(root, 3) && !directFollows) throw new Error('Mobile graph has no discovery root list');
  for (const [owner, targets] of mutedByUser) {
    const accessible = graph.getMutedByUser(str(owner));
    if (accessible.size !== targets.size) throw new Error('Inaccessible mobile graph mute list');
    for (const target of targets) if (!accessible.has(str(target))) throw new Error('Inaccessible mobile graph mute target');
  }
  const revisionCounts = {follows: {ids: 0, complete: 0}, mutes: {ids: 0, complete: 0}};
  for (const revision of graph.getListRevisions()) {
    const count = revision.kind === 3 ? revisionCounts.follows : revisionCounts.mutes;
    count.ids++;
    if (revision.complete) count.complete++;
  }
  return {graph, counts: {users, follows, mutes, directFollows, secondDegree: sizeByDistance[2] ?? 0, revisionCounts}};
}

function checkSource(receipt: MobileGraphReceipt) {
  const s = receipt.source;
  const dates = [s.graphSince, s.graphCompletedAt, s.generatedAt];
  if (receipt.version !== 2 || s.formatVersion !== 4 || !hex.test(receipt.sha256) ||
      s.bytes !== receipt.bytes || s.sha256 !== receipt.sha256 ||
      !hex.test(s.eventId) || !hex.test(s.directoryHash) || nip19.decode(s.publisher).type !== 'npub' ||
      !s.treeName || s.treeName.includes('/') || (s.file !== 'mobile.bin' && s.file !== `mobile-${receipt.root}.bin`) ||
      s.url !== `htree://${s.publisher}/${s.treeName}/${s.file}` ||
      !dates.every(value => Number.isSafeInteger(value) && value >= 0) || !s.graphCompletedAt ||
      s.graphSince > s.graphCompletedAt || s.graphCompletedAt > s.generatedAt) {
    throw new Error('Invalid mobile graph source receipt');
  }
}

/** Verify frozen bytes and their reviewed receipt without opening a relay or a content server. */
export async function verifyMobileGraphSnapshot(bytes: Uint8Array, receipt: MobileGraphReceipt): Promise<SocialGraph> {
  checkBytes(receipt.root, bytes);
  checkSource(receipt);
  if (bytes.length !== receipt.bytes || toHex(await sha256(bytes)) !== receipt.sha256) {
    throw new Error('Mobile graph does not match its pinned checksum');
  }
  const {graph, counts} = await inspect(receipt.root, bytes);
  checkMobileGraphCoverage(counts.users, receipt.source.publisher, receipt.source.treeName);
  for (const field of ['users','follows','mutes','directFollows','secondDegree'] as const) {
    if (counts[field] !== receipt[field]) throw new Error('Mobile graph row count mismatch: ' + field);
  }
  for (const kind of ['follows','mutes'] as const) {
    const actual = counts.revisionCounts[kind];
    const pinned = receipt.revisionCounts[kind];
    if (actual.ids !== pinned.ids || actual.complete !== pinned.complete) throw new Error('Mobile graph revision count mismatch: ' + kind);
  }
  return graph;
}

/** Read one signed generation's selected mobile member. Pass an open snapshot to reuse a preview. */
export async function readMobileGraphSnapshot(root: string, options: GraphSnapshotOptions & {snapshot?: GraphSnapshot; minUsers?: number} = {}) {
  if (!hex.test(root)) throw new Error('Invalid graph discovery root');
  const snapshot = options.snapshot ?? await openGraphSnapshot(options);
  if ((options.publisher !== undefined && options.publisher !== snapshot.publisher) ||
      (options.treeName !== undefined && options.treeName !== snapshot.treeName)) throw new Error('Mobile graph snapshot name mismatch');
  const manifest = snapshot.manifest;
  const file = mobileGraphFile(manifest, root);
  const member = file === 'mobile.bin' ? manifest.files[file] : manifest.mobile_roots![root];
  const bytes = await snapshot.readFile(file, {signal: options.signal, maxBytes: MOBILE_GRAPH_MAX_BYTES});
  const {graph, counts} = await inspect(root, bytes);
  const {publisher, treeName, eventId, directoryHash} = snapshot;
  checkMobileGraphCoverage(counts.users, publisher, treeName, options.minUsers);
  const receipt: MobileGraphReceipt = {
    version: 2, root, bytes: member.size, sha256: member.sha256, ...counts,
    source: {url: `htree://${publisher}/${treeName}/${file}`, publisher, treeName, eventId, directoryHash,
      file, graphSince: manifest.graph_since, graphCompletedAt: manifest.graph_completed_at,
      generatedAt: manifest.generated_at, formatVersion: 4, bytes: member.size, sha256: member.sha256},
  };
  checkSource(receipt);
  return {bytes, graph, receipt};
}
