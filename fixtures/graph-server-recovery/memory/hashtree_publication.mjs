// Run in an isolated network namespace with the qualified graph server and native htree CLI.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { createReadStream, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { createInterface } from 'node:readline';
import { runMemoryCycles } from './server_memory.mjs';
import { runProfileMemory } from './profile_memory_trial.mjs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { parseArgs, promisify } from 'node:util';

const { values, positionals } = parseArgs({ allowPositionals: true, options: {
  htree: { type: 'string' }, 'legacy-htree': { type: 'string' }, dependencies: { type: 'string' }, output: { type: 'string' }, 'memory-seed': { type: 'string' },
  'profile-rows': { type: 'string' }, 'profile-index': { type: 'string' },
} });
assert.equal(positionals.length, 1, 'Pass the already built graph server');
assert(values.htree && values['legacy-htree'] && values.dependencies && values.output, 'Pass --htree, --legacy-htree, --dependencies and --output');
const binary = path.resolve(positionals[0]);
const htree = path.resolve(values.htree);
const legacyHtree = path.resolve(values['legacy-htree']);
const output = path.resolve(values.output);
assert(!existsSync(output), 'Choose a fresh evidence directory');
const require = createRequire(path.resolve(values.dependencies));
const { WebSocketServer } = require('ws');
const { finalizeEvent, generateSecretKey, getPublicKey, nip19, verifyEvent } = require('nostr-tools');
const Fuse = require('fuse.js');
const run = promisify(execFile);
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
async function hashFile(file) {
  const hash = createHash('sha256');
  for await (const bytes of createReadStream(file)) hash.update(bytes);
  return hash.digest('hex');
}
const testKey = name => createHash('sha256').update('graph-memory-fixture-v1/' + name).digest();
const key = values['memory-seed'] ? testKey('root') : generateSecretKey();
const root = getPublicKey(key);
const publishKey = values['memory-seed'] ? testKey('publisher') : generateSecretKey();
const publishPubkey = getPublicKey(publishKey);
const npub = nip19.npubEncode(publishPubkey);
const treeName = 'social-graph';
const uri = 'htree://' + npub + '/' + treeName;
const members = ['allowlist.txt', 'mobile.bin', 'profile-data.json', 'profile-index.json', 'social-graph-v4.bin', 'social-graph.bin'];
const alternateRoot = '336f319763657d6b0e65a5b5876719e8c8dcdcf9396852be71ee26b73368b29b';
const alternateMobile = 'mobile-' + alternateRoot + '.bin';
const entries = [...members, alternateMobile, 'manifest.json'].sort();
const names = [];
const stableFriend = getPublicKey(values['memory-seed'] ? testKey('friend') : generateSecretKey());
const tag = (event, name) => event.tags.find(field => field[0] === name)?.[1];

function generation(name, created_at) {
  const friend = getPublicKey(generateSecretKey());
  const events = [3, 10000, 0].map(kind => finalizeEvent({
    kind, created_at, tags: kind === 0 ? [] : kind === 3 ? [['p', friend], ['p', stableFriend]] : [['p', friend]],
    content: kind === 0 ? JSON.stringify({ name, picture: 'https://example.invalid/test.png' }) : '',
  }, key));
  return { name, friend, events };
}

function matches(event, filter) {
  return (!filter.kinds || filter.kinds.includes(event.kind))
    && (!filter.authors || filter.authors.some(prefix => event.pubkey.startsWith(prefix)))
    && (!filter.ids || filter.ids.some(prefix => event.id.startsWith(prefix)))
    && (filter.since === undefined || filter.since <= event.created_at)
    && (filter.until === undefined || filter.until >= event.created_at)
    && Object.entries(filter).every(([key, values]) => !key.startsWith('#')
      || event.tags.some(([name, value]) => name === key.slice(1) && values.includes(value)));
}

async function relay(events, receive) {
  const ws = new WebSocketServer({ port: 0, host: '127.0.0.1' });
  await once(ws, 'listening');
  const state = { url: 'ws://127.0.0.1:' + ws.address().port, writes: 0, queries: 0, errors: [] };
  ws.on('connection', client => {
    client.on('message', raw => {
      const [type, id, ...filters] = JSON.parse(raw.toString());
      if (type === 'REQ') {
        state.queries++;
        state.onRequest?.(id);
        for (const event of events()) {
          if (filters.some(filter => matches(event, filter))) client.send(JSON.stringify(['EVENT', id, event]));
        }
        client.send(JSON.stringify(['EOSE', id]));
        state.afterEose?.(id);
      } else if (type === 'EVENT') {
        state.writes++;
        try {
          receive(id);
          client.send(JSON.stringify(['OK', id.id, true, '']));
        } catch (error) {
          state.errors.push(error.message);
          client.send(JSON.stringify(['OK', id.id, false, 'invalid test event']));
        }
      }
    });
  });
  state.close = async () => {
    for (const socket of ws.clients) socket.terminate();
    await new Promise(resolve => ws.close(resolve));
  };
  return state;
}

async function blossom() {
  const blobs = new Map();
  const state = { uploads: 0, newDuringFault: 0, denials: 0, reads: 0, suffixedReads: 0, recent: [], fault: false };
  const server = createServer(async (request, response) => {
    const resource = new URL(request.url, 'http://127.0.0.1').pathname;
    const finish = (code, body) => {
      state.recent.push(request.method + ' ' + resource.slice(0, 80) + ' ' + code);
      if (state.recent.length > 24) state.recent.shift();
      response.statusCode = code;
      response.end(body);
    };
    if (resource === '/upload' && request.method === 'PUT') {
      const chunks = [];
      for await (const chunk of request) chunks.push(chunk);
      const bytes = Buffer.concat(chunks);
      const hash = digest(bytes);
      if (request.headers['x-sha-256'] !== hash) return finish(400, 'Hash mismatch');
      const sample = bytes.subarray(0, 256);
      // Match the native random-ingest threshold for untrusted CHK payloads.
      const threshold = sample.length === 256 ? 140 : Math.floor(sample.length * 0.55);
      if (bytes.length < 16 || (sample.length >= 64 && new Set(sample).size < threshold))
        return finish(415, 'Rejected non-CHK payload');
      if (blobs.has(hash)) return finish(200);
      // Accept one new blob before failing the rest. A reader can now see a partial
      // replacement in Blossom, while the published name must still resolve to old data.
      if (state.fault && state.newDuringFault > 0) {
        state.denials++;
        return finish(403, 'Test replacement upload denied');
      }
      if (state.fault) state.newDuringFault++;
      blobs.set(hash, bytes);
      state.uploads++;
      return finish(201, JSON.stringify({ sha256: hash, size: bytes.length, url: state.url + '/' + hash }));
    }
    // The native Blossom clients use /<sha256>.bin for both HEAD and GET.
    // Ordinary BUD-01 clients can request the same content without an extension.
    const hash = /^\/([a-f0-9]{64})(?:\.bin)?$/.exec(resource)?.[1];
    if ((request.method === 'GET' || request.method === 'HEAD') && blobs.has(hash)) {
      const bytes = blobs.get(hash);
      response.setHeader('content-length', bytes.length);
      if (request.method === 'GET') {
        state.reads++;
        if (resource.endsWith('.bin')) state.suffixedReads++;
      }
      return finish(200, request.method === 'GET' ? bytes : undefined);
    }
    finish(404);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  state.url = 'http://127.0.0.1:' + server.address().port;
  state.checkEncrypted = (files, gen) => {
    for (const bytes of blobs.values()) {
      // The relay publishes the decryption key. Public Blossom bytes should still
      // be ciphertext, including the names in the directory node and file contents.
      for (const plaintext of Object.values(files)) assert(!bytes.includes(plaintext), 'Exposed plaintext member');
      assert(!bytes.includes(Buffer.from('social-graph.bin')) && !bytes.includes(Buffer.from(gen.name)));
    }
  };
  state.close = () => new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });
  return state;
}

async function until(check, label, timeout = 45_000) {
  const deadline = Date.now() + timeout;
  do {
    const result = await check();
    if (result) return result;
    await sleep(125);
  } while (Date.now() < deadline);
  throw Error('Timed out waiting for ' + label);
}

function envFor(directory, naming, storage, signer) {
  const env = { PATH: process.env.PATH, HOME: path.join(directory, 'home'), TMPDIR: path.join(directory, 'tmp'),
    HTREE_CONFIG_DIR: path.join(directory, 'config'), HTREE_DATA_DIR: path.join(directory, 'cas') };
  for (const dir of [env.HOME, env.TMPDIR, env.HTREE_CONFIG_DIR, env.HTREE_DATA_DIR]) mkdirSync(dir, { recursive: true, mode: 0o700 });
  writeFileSync(path.join(env.HTREE_CONFIG_DIR, 'config.toml'), [
    // Both CLI versions otherwise launch an unawaited __bg_check on a fresh store.
    '[updater]', 'auto_check = false', 'auto_install = false',
    '[server]', 'public_plaintext_reads = false', '[nostr]', 'bootstrap_follows = []',
    'relays = [' + JSON.stringify(naming.url) + ']', '[blossom]', 'require_random_untrusted_ingest = true',
    'read_servers = [' + JSON.stringify(storage.url) + ']', 'write_servers = [' + JSON.stringify(storage.url) + ']', '',
  ].join('\n'));
  if (signer) writeFileSync(path.join(env.HTREE_CONFIG_DIR, 'keys'), nip19.nsecEncode(signer) + '\n', { mode: 0o600 });
  return env;
}

function signalGroup(pgid, signal) {
  try { process.kill(-pgid, signal); return true; }
  catch (error) { if (error.code === 'ESRCH') return false; throw error; }
}

// A detached crawler owns its session and group. An adopted zombie has exited
// and can only be reaped by the namespace's PID 1 once this fixture returns.
function crawlerMembers(pgid) {
  const result = [];
  for (const pid of readdirSync('/proc')) {
    if (!/^\d+$/.test(pid)) continue;
    let stat;
    try { stat = readFileSync('/proc/' + pid + '/stat', 'utf8'); }
    catch (error) { if (['ENOENT', 'ESRCH'].includes(error.code)) continue; throw error; }
    const [state, parent, group, session] = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    if (+group === pgid && +session === pgid) result.push({ pid: +pid, parent: +parent, state });
  }
  return result;
}

async function waitBriefly(check, ms) {
  const end = Date.now() + ms;
  while (!check() && Date.now() < end) await sleep(40);
  return check();
}

async function launch(directory, env, upstream, storage, enabled, options = {}) {
  const bound = createServer().listen(0, '127.0.0.1');
  await once(bound, 'listening');
  const port = bound.address().port;
  await new Promise(resolve => bound.close(resolve));
  const cgroup = values['profile-rows'] && process.env.PROFILE_MEMORY_CGROUP;
  // The shell moves itself before exec; every native helper inherits the same cgroup.
  const command = cgroup ? '/bin/sh' : binary;
  const argv = cgroup ? ['-ec', 'echo "$$" > "$1/cgroup.procs"; exec "$2"', 'profile-memory', cgroup, binary] : [];
  const child = spawn(command, argv, { detached: true, stdio: ['ignore', 'pipe', 'pipe'], env: {
    ...env, BIND_ADDRESS: '127.0.0.1', PORT: String(port), DATA_DIR: directory, SOCIAL_GRAPH_ROOT: root,
    SOCIAL_GRAPH_DB_DIR: path.join(directory, 'graph'), GRAPH_FORWARD_ALLOWLIST_PATH: path.join(directory, 'allowlist'),
    RELAY_URLS: upstream.url, CRAWL_DISTANCE: '0', PROFILE_CRAWL_DISTANCE: '0', PROFILE_CRAWL_LIMIT: '1',
    SYNC_INTERVAL_MS: '1000', GRAPH_PUBLISH_NPUB: npub, GRAPH_PUBLISH_HTREE_BIN: htree,
    GRAPH_PUBLISH_BLOSSOM_URL: storage.url, GRAPH_PUBLISH_INTERVAL_SECS: '10',
    ...(enabled ? { GRAPH_PUBLISH_HASHTREE: 'true' } : {}), RUST_LOG: 'info', ...options.env,
  } });
  let closed = false;
  child.once('close', () => { closed = true; });
  let log = '';
  for (const stream of [child.stdout, child.stderr]) stream.on('data', data => { log = (log + data.toString()).slice(-12000); });
  const check = () => assert.equal(child.exitCode, null, 'Crawler exited: ' + log);
  const read = async route => {
    const response = await fetch('http://127.0.0.1:' + port + route, { signal: AbortSignal.timeout(2000) });
    assert.equal(response.status, 200, route);
    return Buffer.from(await response.arrayBuffer());
  };
  if (options.onLine) for (const stream of [child.stdout, child.stderr])
    createInterface({ input: stream }).on('line', options.onLine);
  return { read, check, pid: child.pid, origin: `http://127.0.0.1:${port}`, log: () => log,
    async health() {
      try {
        const response = await fetch('http://127.0.0.1:' + port + '/healthz', { signal: AbortSignal.timeout(2000) });
        return await response.json();
      } catch { return null; }
    },
    async stop() {
      const group = child.pid;
      const linux = process.platform === 'linux';
      if (linux) {
        // Give an in-flight df or htree time to finish and its parent to reap it.
        await waitBriefly(() => crawlerMembers(group).every(member => member.pid === group), 1500);
      }
      const stopped = () => closed && (!signalGroup(group, 0) || (linux &&
        crawlerMembers(group).every(member => member.state === 'Z' && member.parent === 1)));
      if (stopped()) return;
      signalGroup(group, 'SIGTERM');
      if (await waitBriefly(stopped, 2000)) return;
      signalGroup(group, 'SIGKILL');
      assert(await waitBriefly(stopped, 3000), 'Crawler session still active: ' +
        JSON.stringify(linux ? crawlerMembers(group) : { group }));
    } };
}

async function waitForProfile(worker, expected) {
  return until(async () => {
    worker.check();
    try {
      const health = JSON.parse(await worker.read('/healthz'));
      const rows = JSON.parse(await worker.read('/profile-data'));
      const index = JSON.parse(await worker.read('/profile-index'));
      return health.healthy && rows.some(([pubkey, name]) => pubkey === root && name === expected.name)
        && index.records.some(record => record.$['0']?.v === expected.name);
    } catch { return false; }
  }, expected.name + ' crawler readiness');
}

async function readNamed(scratch, phase, naming, storage, verifyLarge = false) {
  const home = path.join(scratch, phase);
  const env = envFor(home, naming, storage);
  assert.deepEqual(readdirSync(env.HTREE_DATA_DIR), []);
  const location = path.join(home, 'download');
  const beforeQueries = naming.queries, beforeReads = storage.suffixedReads;
  const opts = { env, encoding: 'buffer', timeout: values['profile-rows'] ? 180_000 : 40_000, maxBuffer: (values['profile-rows'] ? 40 : 2) * 1024 * 1024 };
  const { stderr } = await run(htree, ['--data-dir', env.HTREE_DATA_DIR, 'get', uri, '--non-interactive', '-o', location], opts);
  const resolvedHash = /Resolved to: ([a-f0-9]{64})/.exec(stderr.toString())?.[1];
  const signed = names.find(event => tag(event, 'hash') === resolvedHash);
  assert(signed, 'Reader must resolve an actual signed root from this relay');
  const cid = resolvedHash + ':' + tag(signed, 'key');
  assert(naming.queries > beforeQueries && storage.suffixedReads > beforeReads, 'Fresh reader must resolve the name and retrieve native .bin blob URLs');
  assert.deepEqual(readdirSync(location).sort(), entries, 'Public tree contains exactly the documented members');
  const files = Object.fromEntries(entries.map(name => [name, readFileSync(path.join(location, name))]));
  // Read the same files by immutable CID once a consumer has resolved the name.
  for (const filename of (verifyLarge ? ['profile-index.json'] : ['social-graph.bin', 'profile-index.json', 'allowlist.txt'])) {
    const { stdout } = await run(htree, ['--data-dir', env.HTREE_DATA_DIR, 'cat', cid + '/' + filename], opts);
    if (values['profile-rows']) assert.equal(digest(stdout), digest(files[filename]), 'Verified immutable file');
    else assert.deepEqual(stdout, files[filename], filename + ' immutable read');
  }
  assert(!existsSync(path.join(env.HTREE_DATA_DIR, 'last-update-check')), 'Reader launched a background update check');
  return { files, cid };
}

async function readImmutable(cli, home, naming, storage, { cid, files }) {
  const env = envFor(home, naming, storage);
  const beforeReads = storage.reads;
  const location = path.join(home, 'download');
  // get accepts the verified hash:key from the signed name on both CLI versions.
  await run(cli, ['--data-dir', env.HTREE_DATA_DIR, 'get', cid, '-o', location],
    { env, encoding: 'buffer', timeout: 40_000, maxBuffer: 2 * 1024 * 1024 });
  assert(!existsSync(path.join(env.HTREE_DATA_DIR, 'last-update-check')), 'Immutable reader launched a background update check');
  assert(storage.reads > beforeReads, 'Cold immutable reader must fetch remote ciphertext');
  assert.deepEqual(readdirSync(location).sort(), entries);
  for (const file of entries) assert.deepEqual(readFileSync(path.join(location, file)), files[file], 'Immutable read: ' + file);
}

function checkManifest(files) {
  const manifest = JSON.parse(files['manifest.json']);
  assert.equal(manifest.version, 1);
  assert.equal(manifest.root, root);
  for (const key of ['graph_completed_at', 'graph_since', 'generated_at']) assert(Number.isSafeInteger(manifest[key]));
  assert(manifest.graph_since > 0 && manifest.graph_since <= manifest.graph_completed_at);
  assert(manifest.graph_completed_at <= manifest.generated_at && manifest.generated_at <= Math.floor(Date.now() / 1000) + 1);
  assert.deepEqual(Object.keys(manifest.files).sort(), members);
  for (const file of members) assert.deepEqual(manifest.files[file], { size: files[file].length, sha256: digest(files[file]) });
  const projection = files[alternateMobile];
  assert.deepEqual(manifest.mobile_roots, { [alternateRoot]: {
    file: alternateMobile, size: projection.length, sha256: digest(projection),
  } });
  assert.equal(projection[0], 4);
  assert(projection.length <= 4 * 1024 * 1024);
  return manifest;
}

async function currentConsumerData(worker) {
  const [graph, profiles, index, allowlist] = await Promise.all(['/social-graph', '/profile-data', '/profile-index', '/allowlist'].map(worker.read));
  return { graph, profiles: JSON.parse(profiles), index: JSON.parse(index), allowlist };
}

function checkConsumer(ordinary, files, gen) {
  const canonical = ordinary.graph;
  assert.equal(canonical[0], 3);
  assert.deepEqual(files['social-graph.bin'], canonical, 'Exact graph bytes fetched by the notification consumer');
  for (const file of ['social-graph-v4.bin', 'mobile.bin']) {
    assert.equal(files[file][0], 4);
    for (const event of gen.events.filter(event => event.kind !== 0))
      assert(files[file].includes(Buffer.from(event.id, 'hex')), file + ' must retain signed follow and mute IDs');
  }
  const rows = JSON.parse(files['profile-data.json']);
  const index = JSON.parse(files['profile-index.json']);
  assert.deepEqual(rows, ordinary.profiles);
  assert.deepEqual(index, ordinary.index);
  const docs = rows.map(([pubKey, name, nip05]) => ({ name, pubKey, nip05 }));
  const search = new Fuse(docs, { keys: ['name', 'pubKey', 'nip05'] }, Fuse.parseIndex(index));
  assert.equal(search.search(gen.name)[0]?.item.pubKey, root, 'Named snapshot must support real profile search');
  const allowed = files['allowlist.txt'].toString().trim().split('\n');
  assert(allowed.includes(root) && allowed.includes(gen.friend) && allowed.includes(stableFriend), 'Consumer allowlist must contain root and updated follow');
  assert.deepEqual(files['allowlist.txt'], ordinary.allowlist, 'Preserve exact HTTP allowlist bytes and order across a v4 round trip');
}

const now = Math.floor(Date.now() / 1000);
const first = generation('Initial Fixture Profile', now - 20);
const second = generation('Recovered Fixture Profile', now - 10);
let current = first;
const scratch = mkdtempSync(path.join(tmpdir(), 'named-graph-'));
let worker, upstream, naming, storage;
try {
  upstream = await relay(() => current.events, () => assert.fail('Crawler published to its input relay'));
  storage = await blossom();
  let heldHash;
  naming = await relay(() => names.slice(-1), event => {
    assert(verifyEvent(event), 'Invalid root signature');
    assert.equal(event.kind, 30064);
    assert.equal(event.pubkey, publishPubkey);
    assert.equal(tag(event, 'd'), treeName);
    assert.match(tag(event, 'hash'), /^[a-f0-9]{64}$/);
    assert.match(tag(event, 'key'), /^[a-f0-9]{64}$/, 'Legacy-compatible public CHK key');
    if (storage.fault) assert.equal(tag(event, 'hash'), heldHash, 'Published partially uploaded replacement');
    names.push(event);
  });

  // Even when valid credentials and routes are available, omission of the
  // opt-in should keep the default crawler completely local.
  if (values['memory-seed']) {
    const runTrial = values['profile-rows'] ? runProfileMemory : runMemoryCycles;
    assert.equal(Boolean(values['profile-rows']), Boolean(values['profile-index']));
    await runTrial({ profileRows: values['profile-rows'], profileIndex: values['profile-index'], binary, htree, seed: path.resolve(values['memory-seed']), output, scratch, naming, storage, upstream, names, root, key, publishKey, stableFriend, testKey, getPublicKey, finalizeEvent, verifyEvent, envFor, launch, checkManifest, readNamed, run, hashFile, digest, setEvents: events => { current = { events }; }, track: value => { worker = value; } });
  } else {
  const disabled = path.join(scratch, 'default-off');
  const defaultEnv = envFor(disabled, naming, storage, publishKey);
  worker = await launch(disabled, defaultEnv, upstream, storage, false);
  await waitForProfile(worker, first);
  const requests = upstream.queries;
  await until(() => upstream.queries > requests, 'second disabled crawl cycle');
  assert.equal(naming.writes, 0);
  assert.equal(storage.uploads, 0);
  await worker.stop();
  worker = undefined;

  // A prerequisite graph checkpoint write fails before any snapshot has ever
  // been announced. Keep the same process running long enough for a second try.
  const brokenDirectory = path.join(scratch, 'broken-checkpoint');
  const brokenEnv = envFor(brokenDirectory, naming, storage, publishKey);
  let obstructed = false;
  upstream.onRequest = id => {
    if (!obstructed && String(id).startsWith('graph-crawl-')) {
      mkdirSync(path.join(brokenDirectory, 'graph-freshness.json.tmp'));
      obstructed = true;
    }
  };
  worker = await launch(brokenDirectory, brokenEnv, upstream, storage, true);
  await until(async () => {
    worker.check();
    const health = await worker.health();
    return obstructed && health?.healthy === false && health.freshness.consecutive_graph_failures >= 2;
  }, 'two failed durable crawler checkpoints');
  assert.equal(naming.writes, 0, 'Uncommitted crawl published a root');
  assert.equal(storage.uploads, 0, 'Uncommitted crawl uploaded blobs');
  await worker.stop();
  worker = undefined;
  delete upstream.onRequest;

  const directory = path.join(scratch, 'publisher');
  const env = envFor(directory, naming, storage, publishKey);
  const sentinel = path.join(env.HTREE_DATA_DIR, 'unrelated-file');
  writeFileSync(sentinel, 'retained');
  const writerClean = () => {
    assert.equal(readFileSync(sentinel, 'utf8'), 'retained');
    return !readdirSync(env.HTREE_DATA_DIR).some(entry => entry.startsWith('graph-cas-'));
  };
  worker = await launch(directory, env, upstream, storage, true);
  await waitForProfile(worker, first);
  const ordinary = await currentConsumerData(worker);
  await until(() => { assert.deepEqual(naming.errors, []); return names.length; }, 'first complete published tree');
  heldHash = tag(names.at(-1), 'hash');
  storage.fault = true;
  current = second;
  const initial = await readNamed(scratch, 'first-reader', naming, storage);
  checkManifest(initial.files);
  checkConsumer(ordinary, initial.files, first);
  assert.deepEqual(initial.files[alternateMobile], await worker.read('/social-graph/mobile?root=' + alternateRoot));
  storage.checkEncrypted(initial.files, first);
  assert.equal(initial.cid.split(':')[0], heldHash);

  await waitForProfile(worker, second);
  await until(() => storage.denials && worker.log().includes('named snapshot publication failed') && writerClean(),
    'failed replacement upload and CAS cleanup');
  assert.equal(storage.newDuringFault, 1);
  const during = await readNamed(scratch, 'reader-during-failure', naming, storage);
  assert.equal(during.cid, initial.cid);
  assert.deepEqual(during.files, initial.files, 'Complete previous signed snapshot must survive partial upload');

  storage.fault = false;
  let recovered;
  let seen = heldHash;
  let probe = 0;
  await until(async () => {
    worker.check();
    assert.deepEqual(naming.errors, []);
    const latest = tag(names.at(-1), 'hash');
    if (latest === seen) return false;
    seen = latest;
    const candidate = await readNamed(scratch, 'retry-reader-' + (++probe), naming, storage);
    if (!JSON.parse(candidate.files['profile-data.json']).some((row) => row[0] === root && row[1] === second.name)) return false;
    recovered = candidate;
    return true;
  }, 'automatic replacement recovery', 90_000);
  assert.notEqual(recovered.cid, initial.cid);
  const previous = checkManifest(initial.files), updated = checkManifest(recovered.files);
  assert(updated.generated_at >= previous.generated_at);
  checkConsumer(await currentConsumerData(worker), recovered.files, second);
  assert(!recovered.files['allowlist.txt'].toString().split('\n').includes(first.friend), 'Stale follows must leave the allowlist');
  storage.checkEncrypted(recovered.files, second);
  // Legacy named get ignores configured relays. It can read the current signed
  // hash:key directly. Old immutable links should also survive the name moving.
  await readImmutable(legacyHtree, path.join(scratch, 'legacy-reader'), naming, storage, recovered);
  await readImmutable(htree, path.join(scratch, 'previous-immutable'), naming, storage, initial);
  await until(writerClean, 'clean writer scratch after successful announcement');
  assert.equal(upstream.writes, 0);
  assert.deepEqual(naming.errors, []);

  await worker.stop();
  worker = undefined;
  // A separately managed identity can name a completed upload. The crawler
  // supplies only a public immutable reference after its own upload succeeds.
  const handoff = path.join(scratch, 'handoff');
  const handoffEnv = envFor(handoff, naming, storage, publishKey);
  const receiptPath = path.join(handoff, 'upload.json');
  const previousReceipt = JSON.stringify({ hash: heldHash, key: initial.cid.split(':')[1] });
  writeFileSync(receiptPath, previousReceipt);
  const namesBeforeHandoff = naming.writes, denialsBeforeHandoff = storage.denials;
  storage.fault = true;
  current = generation('Handoff Profile', now);
  worker = await launch(handoff, { ...handoffEnv, GRAPH_PUBLISH_RECEIPT_PATH: receiptPath }, upstream, storage, true);
  await until(() => storage.denials > denialsBeforeHandoff, 'failed upload in handoff mode');
  assert.equal(readFileSync(receiptPath, 'utf8'), previousReceipt);
  storage.fault = false;
  const upload = await until(() => {
    const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
    return receipt.version === 1 ? receipt : null;
  }, 'complete upload receipt', 70_000);
  assert.match(upload.hash, /^[a-f0-9]{64}$/);
  assert.match(upload.key, /^[a-f0-9]{64}$/);
  const cold = envFor(path.join(scratch, 'receipt-reader'), naming, storage);
  const { stdout: manifestBytes } = await run(htree, ['--data-dir', cold.HTREE_DATA_DIR, 'cat',
    upload.hash + ':' + upload.key + '/manifest.json'], {env: cold, encoding: 'buffer', timeout: 40_000});
  assert.equal(upload.manifest_sha256, digest(manifestBytes));
  assert.equal(upload.graph_completed_at, JSON.parse(manifestBytes).graph_completed_at);
  await worker.stop();
  worker = undefined;
  assert.equal(naming.writes, namesBeforeHandoff, 'Receipt mode must leave name signing to its owner');

  mkdirSync(output);
  for (const [stage, generation] of [['initial', initial], ['recovered', recovered]]) {
    const dest = path.join(output, stage);
    mkdirSync(dest);
    for (const file of entries) writeFileSync(path.join(dest, file), generation.files[file], { flag: 'wx' });
  }
  const receipt = { passed: true, binarySha256: await hashFile(binary), htreeSha256: await hashFile(htree),
    legacyHtreeSha256: await hashFile(legacyHtree), legacyReaderVerified: true, encryptedAtRest: true,
    namedTree: uri, defaultOff: true, priorSnapshotPreserved: true, notificationBytesMatched: true,
    readerUsedSeparateStore: true, retryRecovered: true, upstreamUploads: upstream.writes,
    uploadReceiptVerified: true, alternateMobileVerified: true, manifestVerified: true, failedCheckpointStayedPrivate: true, writerScratchCleaned: true, oldImmutableReadableAfterRename: true, acceptedBeforeFailure: storage.newDuringFault, rejectedDuringFailure: storage.denials,
    initialRoot: heldHash, recoveredRoot: recovered.cid.split(':')[0],
    expectedNotifierPolicy: { root, initialFriend: first.friend, recoveredFriend: second.friend, stableFriend },
    initialDirectory: 'initial', recoveredDirectory: 'recovered' };
  writeFileSync(path.join(output, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx' });
  console.log('Named graph publication, cold reads, partial upload recovery and legacy decryption passed.');
  }
} catch (error) {
  // Emit bounded diagnostics in the synthetic namespace. Secret files never enter evidence.
  if (!values['profile-rows']) console.error('Recent synthetic Blossom requests:', storage?.recent, 'accepted uploads:', storage?.uploads, 'successful GETs:', storage?.reads);
  if (!values['profile-rows']) console.error((worker?.log() || '').replace(/nsec1[023456789acdefghjklmnpqrstuvwxyz]+/g, '[redacted]'));
  if (values['profile-rows'] && (error.stdout !== undefined || error.stderr !== undefined || error instanceof SyntaxError)) {
    throw new Error('Profile qualification failed in a native command or JSON parser');
  }
  throw error;
} finally {
  if (worker) await worker.stop();
  for (const service of [upstream, naming, storage]) if (service) await service.close();
  rmSync(scratch, { recursive: true, force: true });
}
