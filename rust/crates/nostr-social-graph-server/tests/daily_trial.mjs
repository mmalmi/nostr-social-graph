// Run inside the normal native graph publication fixture. Only aggregate measurements
// leave the private test directory. Distinct relays supply complementary signed kinds.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { createDailyCorpus } from './daily_corpus.mjs';

const compactRtt = [80, 125, 250, 165, 145];
// The heavy inner tier and faster outer tiers have independently paced EOSE.
const fullRtt = [100, 300, 1800, 350, 300];
const now = () => performance.now();

function crawlerResources(pid) {
  if (process.platform !== 'linux') return {};
  const proc = file => readFileSync('/proc/' + pid + '/' + file, 'utf8');
  const stat = proc('stat');
  const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
  const hz = Number(execFileSync('getconf', ['CLK_TCK'], {encoding:'utf8'}));
  assert(hz > 0);
  const io = proc('io');
  return {
    peakCrawlerRssBytes:Number(/^VmHWM:\s+(\d+) kB/m.exec(proc('status'))[1]) * 1024,
    crawlerCpuMs:(Number(fields[11]) + Number(fields[12])) * 1000 / hz,
    crawlerReadBytes:Number(/^read_bytes:\s+(\d+)$/m.exec(io)[1]),
    crawlerWriteBytes:Number(/^write_bytes:\s+(\d+)$/m.exec(io)[1]),
  };
}

function relayMonitor(corpus, fullSize, ctx) {
  const requests = new Map();
  const timers = new Set();
  const distance = corpus.authorsByDepth.map((_, depth) => ({
    depth, visits: [new Set(), new Set()], requests: 0, emptyRequests: 0, eventsSent: 0,
    completed: 0, maxAuthors: 0, started: Infinity, finished: 0, delayTotal: 0, eventBytes: 0,
    emptyRelayResponses: 0,
  }));
  const profile = ctx.finalizeEvent({kind: 0, created_at: 1690000000, tags: [],
    content: JSON.stringify({name: 'daily fixture'})}, ctx.key);
  let active = 0, maxInFlight = 0, firstRequest = Infinity, lastEose = 0;
  let firstFailure, hold, holdReached = false;
  // Every request on each relay is independently parsed and capped, including empty replies.
  function receive(relayIndex, id, filters, socket) {
    if (!String(id).startsWith('graph-crawl-')) {
      if (String(id).startsWith('profile-crawl-') && filters.some(f => f.authors?.includes(ctx.root)) && relayIndex === 0) {
        socket.send(JSON.stringify(['EVENT', id, profile]));
      }
      socket.send(JSON.stringify(['EOSE', id]));
      return false;
    }
    try {
      assert.equal(filters.length, 1);
      const [f] = filters;
      assert.deepEqual([...f.kinds].sort((a,b) => a-b), [3, 10000]);
      assert(f.authors.length > 0 && f.authors.length <= 32);
      assert.equal(f.limit, f.authors.length * 2);
      assert.equal(f.since, undefined, 'The daily pass must request complete history');
      const depth = corpus.depthByAuthor.get(f.authors[0]);
      const d = distance[depth];
      assert(d && f.authors.every(author => corpus.depthByAuthor.get(author) === depth), 'Unknown or mixed-distance authors');
      const arrived = now();
      firstRequest = Math.min(firstRequest, arrived);
      d.started = Math.min(d.started, arrived);
      let entry = requests.get(id);
      if (!entry) {
        entry = {arrived: new Set(), eose: new Set(), count: 0};
        requests.set(id, entry);
        d.requests++; d.maxAuthors = Math.max(d.maxAuthors, f.authors.length);
        active++; maxInFlight = Math.max(maxInFlight, active);
      }
      assert(!entry.arrived.has(relayIndex), 'Duplicate subscription');
      entry.arrived.add(relayIndex);
      for (const author of f.authors) {
        assert(!d.visits[relayIndex].has(author), 'Duplicate author in one full sweep');
        d.visits[relayIndex].add(author);
      }
      const jitter = parseInt(f.authors[0].slice(0, 2), 16) / 255;
      const delay = Math.round((fullSize ? fullRtt : compactRtt)[depth] * (0.84 + jitter * 0.32) +
        (relayIndex === 1 ? (fullSize && depth === 2 ? 130 : 25) : 0));
      d.delayTotal += delay;
      // Hold both sides of the last *distance*, not of the first finishing batch. Verify
      // independently over HTTP that this partial crawl has not advanced the watermark.
      if (depth === 4 && !holdReached && !fullSize) {
        holdReached = true;
        hold = {id, release: null, promise: null};
        hold.promise = new Promise(resolve => { hold.release = resolve; });
      }
      const timer = setTimeout(async () => {
        timers.delete(timer);
        try {
          if (hold?.id === id) await hold.promise;
          let count = 0;
          for (const author of f.authors) {
            for (const event of corpus.heads(author)) {
              if (event.kind !== (relayIndex === 0 ? 3 : 10000)) continue;
              const frame = JSON.stringify(['EVENT', id, event]);
              socket.send(frame);
              d.eventBytes += Buffer.byteLength(frame);
              count++; d.eventsSent++; entry.count++;
            }
          }
          if (!count) d.emptyRelayResponses++;
          socket.send(JSON.stringify(['EOSE', id]));
          entry.eose.add(relayIndex);
          if (entry.eose.size === 2) {
            if (!entry.count) d.emptyRequests++;
            d.completed++;
            d.finished = now(); lastEose = d.finished;
            active--; requests.delete(id);
          }
        } catch (e) { firstFailure ??= e; }
      }, delay);
      timers.add(timer);
    } catch (e) { firstFailure ??= e; }
    return false;
  }
  return {
    receive, check() { if (firstFailure) throw firstFailure; },
    blocked() { return holdReached; }, release() { hold?.release(); },
    finish() {
      assert.equal(active, 0);
      const stats = distance.map((d,i) => {
        const expected = corpus.authorsByDepth[i].length;
        for (const visits of d.visits) assert.equal(visits.size, expected, 'Missing authors at distance ' + i);
        assert.equal(d.completed, d.requests, 'Unfinished relay replies');
        return {depth:i, authorsVisited:expected, requests:d.requests, emptyRequests:d.emptyRequests,
          eventsSent:d.eventsSent, eventFrameBytes:d.eventBytes, emptyRelayResponses:d.emptyRelayResponses,
          maxAuthors:d.maxAuthors, sweepMs:d.finished-d.started,
          meanScheduledRttMs:d.delayTotal/(d.requests*2)};
      });
      assert(stats.some(d => d.emptyRequests > 0), 'No empty requests exercised');
      return {distance:stats, maxInFlight, crawlMs:lastEose-firstRequest};
    },
    close() { hold?.release(); for (const timer of timers) clearTimeout(timer); },
  };
}

export async function runDailyTrial(ctx, mode) {
  assert(['compact', 'full-size'].includes(mode));
  const fullSize = mode === 'full-size';
  const directory = path.join(ctx.scratch, 'daily-trial');
  mkdirSync(directory);
  const seedPath = path.join(directory, 'seed.bin');
  const preparedAt = now();
  const corpus = await createDailyCorpus({fullSize, rootKey:ctx.key, getPublicKey:ctx.getPublicKey,
    finalizeEvent:ctx.finalizeEvent, seedPath});
  assert.equal(corpus.authorsByDepth[0][0], ctx.root);
  const preparationMs = now()-preparedAt;
  const monitor = relayMonitor(corpus, fullSize, ctx);
  const other = await ctx.relay(() => [], () => assert.fail('Unexpected upstream write'));
  ctx.upstream.onRequest = (...args) => monitor.receive(0,...args);
  other.onRequest = (...args) => monitor.receive(1,...args);
  let worker;
  try {
    const env = ctx.envFor(directory, ctx.naming, ctx.storage, ctx.publishKey);
    let failedCycle = false;
    const harnessCpu = process.cpuUsage();
    const begin = now();
    worker = await ctx.launch(directory, env, ctx.upstream, ctx.storage, true, {
      onLine:line => { if (line.includes('graph cycle remains incomplete')) failedCycle = true; }, env:{
      RELAY_URLS: [ctx.upstream.url, other.url].join(','), CRAWL_DISTANCE:'4', CRAWL_BATCH_SIZE:'500',
      PROFILE_CRAWL_DISTANCE:'0', PROFILE_CRAWL_LIMIT:'1', SYNC_INTERVAL_MS:'86400000',
      GRAPH_PUBLISH_INTERVAL_SECS:'1', LEGACY_SOCIAL_GRAPH_BINARY_PATH:seedPath,
    }});
    ctx.track(worker);
    const deadline = fullSize ? 7200000 : 180000;
    const wait = async (fn, label) => ctx.until(async () => {
      worker.check(); monitor.check(); assert(!failedCycle, 'The full crawl failed before publication'); return fn();
    }, label, Math.max(1, deadline-(now()-begin)));
    if (!fullSize) {
      await wait(() => monitor.blocked(), 'deeper-tier request');
      const status = await wait(() => worker.health(), 'partial health');
      assert(!status.freshness.last_graph_success_at);
      assert(status.freshness.checkpoint_is_partial);
      assert.equal(ctx.names.length, 0, 'Partial publication');
      monitor.release();
    }
    await wait(() => ctx.names.length > 0, 'completed signed graph publication');
    const signedMs = now()-begin;
    const tree = await ctx.readNamed(ctx.scratch, 'daily-fresh-reader', ctx.naming, ctx.storage,
      fullSize ? {timeout:240000, maxBuffer:128 * 1024 ** 2} : {});
    const signedReadMs = now()-begin;
    assert(signedReadMs < deadline);
    const resources = crawlerResources(worker.pid);
    const harnessMeasured = process.cpuUsage(harnessCpu);
    const manifest = ctx.checkManifest(tree.files);
    const health = await worker.health();
    assert(health.healthy && !health.freshness.checkpoint_is_partial);
    assert.equal(manifest.graph_completed_at, health.freshness.last_graph_success_at);
    assert.equal(manifest.graph_completed_at, health.freshness.last_full_reconciliation_at);
    monitor.check();
    const signed = tree.files['social-graph-v4.bin'];
    const verified = corpus.verifySnapshot(signed);
    assert.equal(verified.users, corpus.depthByAuthor.size);
    assert.equal(ctx.upstream.writes + other.writes, 0);
    assert.deepEqual(ctx.upstream.errors.concat(other.errors, ctx.naming.errors), []);
    const metrics = monitor.finish();
    const result = {passed:true, mode, binarySha256:await ctx.hashFile(ctx.binary), preparationMs,
      signedMs, signedReadMs, ...metrics, verified, workload: corpus.summary,
      ...resources, harnessCpuMs:(harnessMeasured.user + harnessMeasured.system) / 1000,
      peakHarnessRssBytes:process.resourceUsage().maxRSS * 1024, signedSnapshotBytes:signed.length,
      treeFileBytes:Object.values(tree.files).reduce((n, file) => n + file.length, 0),
      profile:'Two independent loopback relays, complementary follow/mute list heads and delayed EOSE; wall clock includes crawler startup, commits, signing and independent public-name resolution.'};
    writeFileSync(ctx.output, JSON.stringify(result,null,2)+'\n');
  } finally {
    monitor.close();
    try { if (worker) { await worker.stop(); ctx.track(undefined); } } finally { await other.close(); }
  }
}
