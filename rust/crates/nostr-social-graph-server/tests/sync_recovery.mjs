/** Real loopback relay/server persistence recovery. Never compiles or contacts public relays. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { createWriteStream, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync, cpSync, statfsSync, statSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const args = process.argv.slice(2);
const binaryArgument = args.shift();
assert(binaryArgument && !binaryArgument.startsWith('--'), 'Pass an already qualified server executable');
const binary = path.resolve(binaryArgument);
function option(name, fallback) {
  const index = args.indexOf(name);
  return index < 0 ? fallback : args[index + 1];
}
const here = path.dirname(fileURLToPath(import.meta.url));
const dependencies = path.resolve(option('--dependencies', path.join(here, '../../../../ts/package.json')));
const output = path.resolve(option('--output', 'sync-recovery-evidence'));
assert(statSync(binary).isFile(), 'Server executable must be a regular file');
assert(!existsSync(output), 'Preserve prior fixture evidence; choose a fresh output path');
const require = createRequire(dependencies);
const { WebSocketServer } = require('ws');
const { finalizeEvent, getPublicKey } = require('nostr-tools');
const secret = new Uint8Array(32).fill(0x51);
const root = getPublicKey(secret);
const oldFriend = getPublicKey(new Uint8Array(32).fill(0x52));
const newFriend = getPublicKey(new Uint8Array(32).fill(0x53));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const interval = 1100;
const owned = new Set();
mkdirSync(output, { recursive: true });
const fs = statfsSync(output);
assert(fs.bavail * fs.bsize >= 20 * 1024 ** 3, '20 GiB fixture disk reserve');
const deadline = setTimeout(() => {
  for (const child of owned) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
  throw Error('180 second fixture deadline');
}, 180_000);

function signed(kind, created_at, friend, name) {
  return finalizeEvent({ kind, created_at, content: kind === 0 ? JSON.stringify({ name }) : '',
    tags: kind === 0 ? [] : [['p', friend]] }, secret);
}
function heads(time, friend, name) {
  return [signed(3, time, friend), signed(10000, time, friend), signed(0, time, friend, name)];
}
function matches(event, filter) {
  return (!filter.kinds || filter.kinds.includes(event.kind)) &&
    (!filter.authors || filter.authors.includes(event.pubkey)) &&
    (!filter.ids || filter.ids.includes(event.id)) &&
    (filter.since === undefined || event.created_at >= filter.since);
}
async function until(check, label, timeout = 15_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  throw Error(`Readiness deadline: ${label}`);
}
async function unusedPort() {
  const server = createServer().listen(0, '127.0.0.1'); await once(server, 'listening');
  const port = server.address().port; await new Promise(resolve => server.close(resolve)); return port;
}
async function relay(events, beforeRequest = () => {}) {
  const server = new WebSocketServer({ port: 0, host: '127.0.0.1' }); await once(server, 'listening');
  const state = { events, connections: 0, requests: [], uploads: 0, failures: [] };
  server.on('connection', socket => {
    state.connections++;
    socket.on('message', raw => {
      try {
        const frame = JSON.parse(raw.toString());
        if (frame[0] === 'EVENT') { state.uploads++; return; }
        if (frame[0] !== 'REQ') return;
        const request = { id: frame[1], filters: frame.slice(2), at: performance.now() };
        state.requests.push(request); beforeRequest(request);
        for (const event of state.events) if (request.filters.some(filter => matches(event, filter))) {
          socket.send(JSON.stringify(['EVENT', request.id, event]));
        }
        socket.send(JSON.stringify(['EOSE', request.id]));
      } catch (error) { state.failures.push(String(error)); socket.close(); }
    });
  });
  return { ...state, state, url: `ws://127.0.0.1:${server.address().port}`,
    async close() { for (const socket of server.clients) socket.terminate(); await new Promise(resolve => server.close(resolve)); } };
}
async function launch(directory, upstream) {
  const port = await unusedPort();
  // Explicit synthetic environment: never inherit production settings or credentials.
  const env = { PATH: process.env.PATH, HOME: path.join(directory, 'home'),
    TMPDIR: path.join(directory, 'tmp'), XDG_CONFIG_HOME: path.join(directory, 'config') };
  for (const key of ['HOME', 'TMPDIR', 'XDG_CONFIG_HOME']) mkdirSync(env[key], { recursive: true });
  Object.assign(env, { BIND_ADDRESS: '127.0.0.1', PORT: String(port), DATA_DIR: directory,
    SOCIAL_GRAPH_DB_DIR: path.join(directory, 'graph'), SOCIAL_GRAPH_ROOT: root,
    LEGACY_SOCIAL_GRAPH_BINARY_PATH: path.join(directory, 'absent-legacy.bin'),
    RELAY_URLS: upstream.url, CRAWL_DISTANCE: '0', PROFILE_CRAWL_DISTANCE: '0',
    PROFILE_CRAWL_LIMIT: '1', SYNC_INTERVAL_MS: String(interval),
    GRAPH_FORWARD_ALLOWLIST_PATH: path.join(directory, 'allowlist'), RUST_LOG: 'info' });
  const log = createWriteStream(path.join(directory, 'server.log'), { flags: 'wx' });
  const child = spawn(binary, [], { env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] }); owned.add(child);
  let text = ''; child.stdout.on('data', data => { text += data; log.write(data); }); child.stderr.on('data', data => { text += data; log.write(data); });
  const closed = new Promise(resolve => child.once('close', resolve));
  const request = async pathname => {
    const response = await fetch(`http://127.0.0.1:${port}${pathname}`, { signal: AbortSignal.timeout(1000) });
    return pathname === '/healthz' ? response.json() : Buffer.from(await response.arrayBuffer());
  };
  return { child, request, log: () => text,
    async stop() {
      if (child.exitCode === null) { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
      const kill = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }, 3000);
      await closed; clearTimeout(kill); owned.delete(child); await new Promise(resolve => log.end(resolve));
    } };
}
async function health(server) { try { return await server.request('/healthz'); } catch { return null; } }
const graphRequests = relay => relay.state.requests.filter(request => String(request.id).startsWith('graph-crawl-'));
const isProfileRequest = request => request.filters.some(filter => filter.kinds?.includes(0) && filter.authors?.includes(root));
const profileRequests = relay => relay.state.requests.filter(isProfileRequest);
// This deliberately tiny synthetic graph is one unencrypted Hashtree blob.
// Verify its content address, so durable IDs are tested independently of memory.
function storedSnapshot(directory) {
  const manifest = JSON.parse(readFileSync(path.join(directory, 'social-graph-root.json'), 'utf8'));
  assert.match(manifest.cid, /^[0-9a-f]{64}$/);
  const bytes = readFileSync(path.join(directory, 'blobs', manifest.cid.slice(0, 2), manifest.cid.slice(2, 4), manifest.cid.slice(4)));
  assert.equal(sha(bytes), manifest.cid); assert.equal(bytes.length, manifest.size);
  return bytes;
}
function snapshotHas(bytes, events) { return bytes[0] === 4 && events.filter(event => event.kind !== 0).every(event => bytes.includes(Buffer.from(event.id, 'hex'))); }
function contentsContain(file, name) { return existsSync(file) && readFileSync(file, 'utf8').includes(name); }
function storedDigest(target) {
  if (statSync(target).isFile()) return sha(readFileSync(target));
  const entries = [];
  function visit(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const file = path.join(directory, entry.name);
      if (entry.isDirectory()) visit(file);
      else { assert(entry.isFile(), 'Synthetic checkpoint contains only regular files'); entries.push([path.relative(target, file), sha(readFileSync(file))]); }
    }
  }
  visit(target); return sha(JSON.stringify(entries));
}
const results = [];
let activeServer, activeRelay;
try {
  const baseline = path.join(output, 'baseline'); mkdirSync(baseline);
  const old = heads(Math.floor(Date.now() / 1000) - 10, oldFriend, 'before-recovery');
  activeRelay = await relay(old); activeServer = await launch(baseline, activeRelay);
  await until(async () => (await health(activeServer))?.healthy && contentsContain(path.join(baseline, 'profileData.large.json'), 'before-recovery'), 'baseline durable graph/profile');
  const previous = storedSnapshot(path.join(baseline, 'graph')); assert(snapshotHas(previous, old));
  const previousLegacy = await activeServer.request('/social-graph'); assert.equal(previousLegacy[0], 3);
  writeFileSync(path.join(output, 'last-good-snapshot.bin'), previous);
  await activeServer.stop(); await activeRelay.close(); activeServer = activeRelay = null;

  for (const fault of ['checkpoint', 'freshness', 'allowlist', 'profile', 'profile-index']) {
    const directory = path.join(output, fault); cpSync(baseline, directory, { recursive: true }); rmSync(path.join(directory, 'server.log'));
    const fresh = heads(Math.floor(Date.now() / 1000), newFriend, `after-${fault}`);
    const isProfile = fault.startsWith('profile');
    let injected = false, held, blocked, heldDigest;
    activeRelay = await relay(isProfile ? [old[0], old[1], fresh[2]] : old, request => {
      const relevant = isProfile ? isProfileRequest(request) : String(request.id).startsWith('graph-crawl-');
      if (injected || !relevant) return;
      blocked = path.join(directory, fault === 'checkpoint' ? 'graph' : fault === 'freshness' ? 'graph-freshness.json.tmp' : fault === 'allowlist' ? 'allowlist.tmp' : fault === 'profile' ? 'profileData.large.json' : 'profileIndex.json');
      held = `${blocked}-held`;
      if (existsSync(blocked)) { heldDigest = storedDigest(blocked); renameSync(blocked, held); }
      if (fault === 'checkpoint') writeFileSync(blocked, 'synthetic checkpoint obstruction'); else mkdirSync(blocked);
      injected = true;
    });
    activeServer = await launch(directory, activeRelay);
    const server = activeServer, upstream = activeRelay;
    const first = await until(async () => await health(server), `${fault} HTTP readiness`);
    const pid = server.child.pid, startedAt = first.freshness.started_at;
    await until(async () => {
      assert.equal(server.child.exitCode, null, `${fault}: server exited`);
      if (!injected) return false;
      if (isProfile) return profileRequests(upstream).length >= 2 && server.log().includes('profile cycle remains incomplete');
      return (await health(server))?.freshness.consecutive_graph_failures >= 2;
    }, `${fault} repeated bounded failure`);
    const failed = await health(server);
    if (!isProfile) {
      assert.equal(failed.healthy, false); assert.equal(failed.freshness.graph_since, null);
      assert.equal(failed.freshness.last_graph_success_at, null); assert.equal(failed.freshness.last_full_reconciliation_at, null);
      assert(graphRequests(upstream).every(request => request.filters.every(filter => filter.since === undefined)));
    } else {
      const requests = profileRequests(upstream);
      assert.equal(requests[0].filters[0].since, requests[1].filters[0].since, 'Profile cursor advanced before durable save');
      assert.equal(storedDigest(held), heldDigest, 'Last good profile/index changed');
    }
    assert.deepEqual(await server.request('/social-graph'), previousLegacy, 'Personal graph changed during failure');
    assert(snapshotHas(storedSnapshot(fault === 'checkpoint' ? held : path.join(directory, 'graph')), old), 'Stored personal signed revisions lost during failure');
    if (heldDigest) assert.equal(storedDigest(held), heldDigest, 'Last good stored data changed during failure');
    rmSync(blocked, { recursive: true }); if (existsSync(held)) renameSync(held, blocked);
    upstream.state.events = fresh;
    await until(async () => (await health(server))?.healthy && snapshotHas(storedSnapshot(path.join(directory, 'graph')), fresh) &&
      contentsContain(path.join(directory, 'profileData.large.json'), `after-${fault}`) && contentsContain(path.join(directory, 'profileIndex.json'), `after-${fault}`), `${fault} automatic durable recovery`);
    const recovered = await health(server);
    // Query mobile v4 only now: the endpoint intentionally caches each root for60s.
    assert(snapshotHas(await server.request('/social-graph/mobile'), fresh), 'Recovered mobile snapshot lacks signed IDs');
    assert.equal(server.child.pid, pid); assert.equal(server.child.exitCode, null); assert.equal(recovered.freshness.started_at, startedAt);
    assert.equal(recovered.freshness.consecutive_graph_failures, 0); assert(recovered.freshness.graph_since !== null); assert(recovered.freshness.last_full_reconciliation_at !== null);
    const allowlist = readFileSync(path.join(directory, 'allowlist'), 'utf8'); assert(allowlist.includes(newFriend)); assert(!allowlist.includes(oldFriend));
    assert.equal(upstream.state.connections, 1, 'Duplicate worker/relay connection'); assert.equal(upstream.state.uploads, 0); assert.deepEqual(upstream.state.failures, []);
    const requests = graphRequests(upstream); assert(requests.length >= (fault === 'allowlist' ? 2 : 3) && requests.length < 10);
    for (let index = 1; index < requests.length; index++) assert(requests[index].at - requests[index - 1].at >= interval - 100, 'Busy retry or duplicate sync worker');
    results.push({ fault, pid, startedAt, graphRequests: requests.length, connections: upstream.state.connections, failedFreshness: failed.freshness, recoveredFreshness: recovered.freshness, signedIds: fresh.slice(0, 2).map(event => event.id), automaticRecovery: true });
    await server.stop(); await upstream.close(); activeServer = activeRelay = null;
  }
  writeFileSync(path.join(output, 'result.json'), JSON.stringify({ passed: true, binary, binarySha256: sha(readFileSync(binary)), dependencies, intervalMs: interval, checkpointReserveUnchanged: true, results }, null, 2));
  console.log(JSON.stringify({ passed: true, cases: results.length, output }));
} finally {
  if (activeServer) await activeServer.stop(); if (activeRelay) await activeRelay.close(); clearTimeout(deadline);
}
