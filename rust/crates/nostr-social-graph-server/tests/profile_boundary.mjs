import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync, utimesSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { setTimeout as sleep } from 'node:timers/promises';

export async function runProfileBoundary(ctx, expectStall) {
  const { binary, scratch, output, root, key, publishKey, stableFriend, upstream, naming, storage,
    names, tag, finalizeEvent, generateSecretKey, getPublicKey, Fuse } = ctx;
  assert.equal(process.platform, 'linux');
  const interfaces = readFileSync('/proc/net/dev', 'utf8').split('\n').filter(row => row.includes(':')).map(row => row.split(':')[0].trim());
  assert.deepEqual(interfaces, ['lo'], 'Use the established isolated fixture namespace');
  const started = performance.now();
  const epoch = Math.floor(Date.now() / 1000);
  const earlyKey = generateSecretKey(), lateKey = generateSecretKey(), newcomerKey = generateSecretKey();
  const early = getPublicKey(earlyKey), late = getPublicKey(lateKey), newcomer = getPublicKey(newcomerKey);
  const sign = (kind, signer, created_at, targets = [], name) => finalizeEvent({
    kind, created_at, tags: targets.map(pubkey => ['p', pubkey]), content: name ? JSON.stringify({ name }) : '',
  }, signer);
  const graphEvents = [sign(3, key, epoch - 120, [early, late, stableFriend]), sign(10000, key, epoch - 120, [late])];
  const history = [
    sign(0, key, epoch - 1800, [], 'Recovered Root'),
    sign(0, earlyKey, epoch - 1800, [], 'Recovered Early'),
    sign(0, lateKey, epoch - 1800, [], 'Recovered Late'),
    sign(0, newcomerKey, epoch - 1800, [], 'Recovered Newcomer'),
  ];
  ctx.setEvents([...graphEvents, ...history]);
  const directory = path.join(scratch, 'profile-boundary');
  const env = ctx.envFor(directory, naming, storage, publishKey);
  const profileFile = path.join(directory, 'profileData.large.json');
  writeFileSync(profileFile, JSON.stringify([[root, 'Retained Root'], [early, 'Retained Early'], [late, 'Retained Late']]));
  utimesSync(profileFile, epoch - 3600, epoch - 3600);
  const timing = { PROFILE_CRAWL_DISTANCE: '1', PROFILE_CRAWL_LIMIT: '20', PROFILE_BATCH_SIZE: '1',
    SYNC_INTERVAL_MS: '1000', GRAPH_PUBLISH_INTERVAL_SECS: '1' };
  let worker, hold = 'all', phase = 'initial', graphCycles = 0, profileCycles = 0;
  const requests = [], completeTimes = [], logs = [];
  upstream.onRequest = (id, filters, socket) => {
    const profile = filters.find(filter => filter.kinds?.length === 1 && filter.kinds[0] === 0 && filter.authors?.length);
    if (!profile) return;
    requests.push({ id, phase, atMs: performance.now() - started, filter: profile });
    const withheld = hold === 'all' || (hold === 'late' && profile.authors.includes(late));
    if (withheld) return false;
  };
  function observe(line) {
    logs.push(line.slice(-500));
    if (logs.length > 80) logs.shift();
    if (line.includes('sync cycle finished: graph_committed=true')) {
      graphCycles++;
      const stamp = /\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z/.exec(line)?.[0];
      if (stamp) completeTimes.push(Math.floor(Date.parse(stamp) / 1000));
    }
    if (line.includes('profile cycle finished: profiles_committed=true')) profileCycles++;
  }
  async function wait(check, label, timeout = 20_000) {
    return ctx.until(async () => { worker?.check(); return check(); }, label, timeout);
  }
  async function launch() {
    worker = await ctx.launch(directory, env, upstream, storage, true, { env: timing, onLine: observe });
    ctx.track(worker);
  }
  const rows = files => JSON.parse(files['profile-data.json']);
  function checkTree(generation, expectedNames) {
    const manifest = ctx.checkManifest(generation.files);
    const profileRows = rows(generation.files), index = JSON.parse(generation.files['profile-index.json']);
    const docs = profileRows.map(([pubKey, name, nip05]) => ({ name, pubKey, nip05 }));
    const search = new Fuse(docs, { keys: ['name', 'pubKey', 'nip05'] }, Fuse.parseIndex(index));
    assert.equal(index.records.length, profileRows.length, 'Each serialized profile has one indexed record');
    for (const [pubkey, name] of expectedNames) {
      assert(profileRows.some(row => row[0] === pubkey && row[1] === name), 'Missing expected profile ' + name);
      assert.equal(search.search(name)[0]?.item.pubKey, pubkey, 'Profile index and rows disagree for ' + name);
    }
    assert(completeTimes.some(at => Math.abs(at - manifest.graph_completed_at) <= 1), 'Manifest must identify an observed completed graph crawl');
    return manifest;
  }
  async function observeHealth() {
    const health = await worker.health();
    if (health?.freshness?.last_graph_success_at) completeTimes.push(health.freshness.last_graph_success_at);
    return health;
  }
  async function nextTree(label, predicate, timeout = 25_000) {
    let last, count = 0, found;
    await wait(async () => {
      await observeHealth();
      const hash = tag(names.at(-1) ?? { tags: [] }, 'hash');
      if (!hash || hash === last) return false;
      last = hash;
      const candidate = await ctx.readNamed(scratch, label + '-' + (++count), naming, storage);
      if (!predicate(candidate.files)) return false;
      found = candidate;
      return true;
    }, label, timeout);
    return found;
  }
  function includesHeads(files, events) {
    return ['social-graph-v4.bin', 'mobile.bin'].every(name => events.every(event => files[name].includes(Buffer.from(event.id, 'hex'))));
  }
  function hasNames(files, expected) {
    const data = rows(files);
    return expected.every(([pubkey, name]) => data.some(row => row[0] === pubkey && row[1] === name));
  }
  mkdirSync(output);
  try {
    await launch();
    const firstHealthy = await wait(async () => {
      const health = await observeHealth();
      return health?.healthy && requests.length && health;
    }, 'durable graph and an in-flight delayed profile request');
    const completed = firstHealthy.freshness.last_graph_success_at;
    const beforePublication = performance.now();
    if (expectStall) {
      await sleep(8000);
      assert.equal(names.length, 0, 'Old producer unexpectedly publishes while profile responses are held');
      assert(existsSync(path.join(directory, 'graph', 'social-graph-root.json')));
      assert(requests.length > 0 && (await worker.health()).healthy);
      const receipt = { passed: true, expectProfileStall: true, expectedOldFailure: true, binarySha256: await ctx.hashFile(binary),
        graphCompletedAt: completed, profileRequests: requests, observationMs: performance.now() - beforePublication,
        reason: 'Completed graph remains unpublished while a profile fetch lacks EOSE' };
      writeFileSync(path.join(output, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n');
      return;
    }
    await wait(() => names.length, 'publication independent of the delayed profile response', 8000);
    const firstPublicationMs = performance.now() - beforePublication;
    const first = await ctx.readNamed(scratch, 'delayed-initial', naming, storage);
    await observeHealth();
    const firstManifest = checkTree(first, [[root, 'Retained Root'], [late, 'Retained Late']]);
    assert.equal(firstManifest.graph_completed_at >= completed, true);
    assert(includesHeads(first.files, graphEvents));
    assert.equal(profileCycles, 0, 'Incomplete profile sweep was reported complete');

    const changed = [sign(3, key, Math.floor(Date.now() / 1000), [early, late, stableFriend, newcomer]),
      sign(10000, key, Math.floor(Date.now() / 1000), [newcomer])];
    ctx.setEvents([...changed, ...history]);
    changed.forEach(upstream.broadcast);
    const graphCyclesAtChange = graphCycles;
    const liveAt = Math.floor(Date.now() / 1000);
    await wait(async () => (await observeHealth())?.freshness.last_live_event_at >= liveAt, 'live graph input during profile wait');
    const updated = await nextTree('delayed-live-graph', files => includesHeads(files, changed), 12_000);
    const updatedManifest = checkTree(updated, [[root, 'Retained Root'], [late, 'Retained Late']]);
    assert(updatedManifest.graph_completed_at >= firstManifest.graph_completed_at);
    await wait(() => graphCycles >= graphCyclesAtChange + 2, 'next graph crawls while profiles remain delayed', 12_000);
    await wait(() => requests.filter(r => r.filter.authors.includes(root)).length >= 2,
      'profile timeout retries from its uncommitted cursor', 16_000);
    const initialRetries = requests.filter(r => r.phase === 'initial' && r.filter.authors.includes(root));
    assert(initialRetries.length >= 2);
    assert(initialRetries.every(r => r.filter.since === initialRetries[0].filter.since), 'Timeout advanced the profile cursor');
    assert.equal(profileCycles, 0);
    const cyclesDuringProfileWait = graphCycles;

    phase = 'recovered';
    hold = null;
    upstream.release();
    const expected = [[root, 'Recovered Root'], [early, 'Recovered Early'], [late, 'Recovered Late'], [newcomer, 'Recovered Newcomer']];
    const recovered = await nextTree('profile-complete', files => hasNames(files, expected), 35_000);
    checkTree(recovered, expected);
    await wait(() => profileCycles > 0 && readFileSync(profileFile, 'utf8').includes('Recovered Late'), 'durable completed profile sweep');
    assert(includesHeads(recovered.files, changed));
    assert(requests.some(r => r.phase === 'recovered' && r.filter.authors.includes(newcomer) && r.filter.since === undefined),
      'New author history must be fetched without a since cursor');

    phase = 'late-timeout';
    hold = 'late';
    const newerHistory = [sign(0, key, epoch - 100, [], 'Later Root'), sign(0, earlyKey, epoch - 100, [], 'Later Early'),
      sign(0, lateKey, epoch - 100, [], 'Later Late'), history[3]];
    ctx.setEvents([...changed, ...newerHistory]);
    await wait(() => requests.filter(r => r.phase === phase && r.filter.authors.includes(late)).length >= 2,
      'failed late profile batch retries the same completed-sweep cursor', 26_000);
    const lateRetries = requests.filter(r => r.phase === phase && r.filter.authors.includes(late));
    assert(lateRetries.every(r => r.filter.since === lateRetries[0].filter.since), 'A failed later batch advanced the profile cursor');
    assert(Number.isSafeInteger(lateRetries[0].filter.since), 'Exercise the incremental cursor after a successful sweep');
    assert(lateRetries[0].filter.since <= epoch - 100);
    const cyclesBeforeRestart = graphCycles;
    await worker.stop();
    worker = undefined;
    ctx.track(undefined);
    phase = 'restart';
    hold = null;
    await launch();
    const restartedExpected = [[root, 'Later Root'], [early, 'Later Early'], [late, 'Later Late'], [newcomer, 'Recovered Newcomer']];
    const restarted = await nextTree('restart-replay', files => hasNames(files, restartedExpected), 35_000);
    checkTree(restarted, restartedExpected);
    assert(requests.some(r => r.phase === 'restart' && r.filter.authors.includes(late) && r.filter.since === undefined),
      'Restart must repair delayed historical profiles independently of file modification time');
    assert(includesHeads(restarted.files, changed));
    await ctx.readImmutable(ctx.htree, path.join(scratch, 'old-profile-generation'), naming, storage, first);
    assert.equal(upstream.writes, 0);
    assert.deepEqual(naming.errors, []);
    for (const [label, generation] of [['initial', first], ['live', updated], ['recovered', recovered], ['restarted', restarted]]) {
      const destination = path.join(output, label);
      mkdirSync(destination);
      for (const [name, bytes] of Object.entries(generation.files)) writeFileSync(path.join(destination, name), bytes);
    }
    const receipt = { passed: true, expectProfileStall: false, binarySha256: await ctx.hashFile(binary), firstPublicationMs,
      cyclesDuringProfileWait, cyclesBeforeRestart, graphCycles, profileCycles, graphCompletedAt: completed,
      initialManifest: firstManifest, updatedManifest, profileRequests: requests,
      signedGenerations: names.map(e => ({ id: e.id, hash: tag(e, 'hash'), created_at: e.created_at })),
      followMuteIds: changed.map(e => e.id), independentProfileCursor: true, timeoutRetriesSameCursor: true,
      restartRepairsHistoricalProfiles: true, coherentSevenMembers: true, oldImmutableRead: true,
      upstreamWrites: upstream.writes, elapsedMs: performance.now() - started };
    writeFileSync(path.join(output, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n');
  } catch (error) {
    writeFileSync(path.join(output, 'failure.json'), JSON.stringify({ reason: String(error), graphCycles,
      profileCycles, requests, names: names.length, logs }, null, 2) + '\n');
    throw error;
  } finally {
    if (worker) await worker.stop();
    ctx.track(undefined);
  }
}
