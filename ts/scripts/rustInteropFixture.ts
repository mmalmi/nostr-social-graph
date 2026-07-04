import fs from 'fs';

import { SocialGraph } from '../src/SocialGraph';
import type { NostrEvent } from '../src/utils';

const pubKeys = {
  adam: '020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e',
  fiatjaf: '3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d',
  snowden: '84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240',
  sirius: '4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0',
  bob: '4132aeeee5c7b3497d260c922758e804a9cf9c0933d3e333bfd15f7695db3852',
  charlie: 'abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890',
} as const;

const arbitraryIds = {
  root: 'local:root',
  identity: '6b7f5df4-1d2d-43a7-9b87-873e41a2d99a',
  external: 'external:nvpn-peer:exit-a',
} as const;

type GraphSummary = {
  root: string;
  binary_hex?: string;
  distances: Array<[string, number]>;
  follows: Array<[string, string[]]>;
  followers: Array<[string, string[]]>;
  mutes: Array<[string, string[]]>;
  muters: Array<[string, string[]]>;
  follow_list_created_at: Array<[string, number | null]>;
  mute_list_created_at: Array<[string, number | null]>;
};

function owners() {
  return [pubKeys.adam, pubKeys.bob, pubKeys.fiatjaf];
}

function keyedUsers() {
  return [
    pubKeys.adam,
    pubKeys.bob,
    pubKeys.fiatjaf,
    pubKeys.snowden,
    pubKeys.sirius,
    pubKeys.charlie,
  ];
}

function event(pubkey: string, kind: number, createdAt: number, tagged: string[]): NostrEvent {
  return {
    created_at: createdAt,
    content: '',
    tags: tagged.map((pk) => ['p', pk]),
    kind,
    pubkey,
    id: `${pubkey}:${kind}:${createdAt}`,
    sig: '00'.repeat(64),
  };
}

function defaultGraph() {
  const graph = new SocialGraph(pubKeys.adam);
  const events = [
    event(pubKeys.adam, 3, 1000, [pubKeys.bob, pubKeys.fiatjaf]),
    event(pubKeys.fiatjaf, 3, 1100, [pubKeys.snowden]),
    event(pubKeys.bob, 10000, 1200, [pubKeys.snowden]),
    event(pubKeys.adam, 10000, 1300, [pubKeys.charlie]),
    event(pubKeys.adam, 10000, 900, [pubKeys.snowden]),
    event(pubKeys.fiatjaf, 3, 1400, [pubKeys.sirius]),
  ];
  for (const ev of events) {
    graph.handleEvent(ev, true, 1);
  }
  return graph;
}

function arbitraryIdsGraph() {
  const graph = new SocialGraph(arbitraryIds.root);
  graph.addFollower(arbitraryIds.root, arbitraryIds.identity);
  graph.addFollower(arbitraryIds.identity, arbitraryIds.external);
  return graph;
}

function summary(graph: SocialGraph, binaryHex?: string): GraphSummary {
  return {
    root: graph.getRoot(),
    binary_hex: binaryHex,
    distances: keyedUsers().map((user) => [user, graph.getFollowDistance(user)]),
    follows: owners().map((user) => [user, Array.from(graph.getFollowedByUser(user)).sort()]),
    followers: keyedUsers().map((user) => [user, Array.from(graph.getFollowersByUser(user)).sort()]),
    mutes: owners().map((user) => [user, Array.from(graph.getMutedByUser(user)).sort()]),
    muters: keyedUsers().map((user) => [user, Array.from(graph.getUserMutedBy(user)).sort()]),
    follow_list_created_at: owners().map((user) => [user, graph.getFollowListCreatedAt(user) ?? null]),
    mute_list_created_at: owners().map((user) => [user, graph.getMuteListCreatedAt(user) ?? null]),
  };
}

async function emitScenario(name: string) {
  if (name === 'empty') {
    const graph = new SocialGraph(pubKeys.adam);
    const binary = await graph.toBinary();
    process.stdout.write(JSON.stringify(summary(graph, Buffer.from(binary).toString('hex'))));
    return;
  }

  if (name === 'default') {
    const graph = defaultGraph();
    const binary = await graph.toBinary();
    process.stdout.write(JSON.stringify(summary(graph, Buffer.from(binary).toString('hex'))));
    return;
  }

  if (name === 'arbitrary-ids') {
    const graph = arbitraryIdsGraph();
    const binary = await graph.toBinary();
    process.stdout.write(JSON.stringify(summary(graph, Buffer.from(binary).toString('hex'))));
    return;
  }

  throw new Error(`unknown scenario ${name}`);
}

async function emitBudgetedScenario(
  name: string,
  maxNodes?: number,
  maxEdges?: number,
  maxDistance?: number,
  maxEdgesPerNode?: number
) {
  let graph: SocialGraph;
  if (name === 'empty') {
    graph = new SocialGraph(pubKeys.adam);
  } else if (name === 'default') {
    graph = defaultGraph();
  } else if (name === 'arbitrary-ids') {
    graph = arbitraryIdsGraph();
  } else {
    throw new Error(`unknown scenario ${name}`);
  }

  const binary = await graph.toBinary(maxNodes, maxEdges, maxDistance, maxEdgesPerNode);
  const reconstructed = await SocialGraph.fromBinary(graph.getRoot(), binary);
  process.stdout.write(JSON.stringify(summary(reconstructed, Buffer.from(binary).toString('hex'))));
}

async function loadBinary(root: string, filePath: string) {
  const data = fs.readFileSync(filePath);
  const graph = await SocialGraph.fromBinary(root, new Uint8Array(data));
  process.stdout.write(JSON.stringify(summary(graph)));
}

async function loadBinaryFromUrl(root: string, url: string) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`HTTP error ${response.status} while fetching ${url}`);
  }
  if (!response.body) {
    throw new Error(`response body missing for ${url}`);
  }

  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let totalLength = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    chunks.push(value);
    totalLength += value.length;
  }

  const combined = new Uint8Array(totalLength);
  let offset = 0;
  for (const chunk of chunks) {
    combined.set(chunk, offset);
    offset += chunk.length;
  }

  const graph = await SocialGraph.fromBinary(root, combined);
  await graph.recalculateFollowDistances(1000, Number.MAX_SAFE_INTEGER, () => {});
  process.stdout.write(JSON.stringify(summary(graph)));
}

function parseOptionalNumber(value: string | undefined): number | undefined {
  if (value === undefined || value === '') {
    return undefined;
  }
  return Number(value);
}

const [command, ...args] = process.argv.slice(2);
if (command === 'emit') {
  await emitScenario(args[0] ?? 'default');
} else if (command === 'emit-budget') {
  const [name = 'default', maxNodes, maxEdges, maxDistance, maxEdgesPerNode] = args;
  await emitBudgetedScenario(
    name,
    parseOptionalNumber(maxNodes),
    parseOptionalNumber(maxEdges),
    parseOptionalNumber(maxDistance),
    parseOptionalNumber(maxEdgesPerNode)
  );
} else if (command === 'load') {
  const [root, filePath] = args;
  if (!root || !filePath) {
    throw new Error('usage: load <root> <filePath>');
  }
  await loadBinary(root, filePath);
} else if (command === 'fetch') {
  const [root, url] = args;
  if (!root || !url) {
    throw new Error('usage: fetch <root> <url>');
  }
  await loadBinaryFromUrl(root, url);
} else {
  throw new Error(
    'usage: emit <scenario> | emit-budget <scenario> [maxNodes] [maxEdges] [maxDistance] [maxEdgesPerNode] | load <root> <filePath> | fetch <root> <url>'
  );
}
