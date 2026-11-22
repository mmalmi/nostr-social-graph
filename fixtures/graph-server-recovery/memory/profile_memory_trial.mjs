// Exercise the ordinary producer against matching immutable public graph and profile files.
// Keep raw snapshots in the disposable namespace. The receipt contains only aggregate data.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, statSync, watch, writeFileSync } from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { memorySampler } from './profile_memory_sampler.mjs';

const G = { bytes:55_750_905, sha:'aeae583ae888f7fec2052f678ff30ae2b031ed4e8d77d0fe0c644e35c2bd6fc0' };
const R = { bytes:26_833_179, sha:'66dd586d871663a1ae2162039ad719fbe4f8840dbe4ef058a04bb771f6f9f66f' };
const I = { bytes:26_356_229, sha:'67b9aca2a1adcec4335456a7e67ca01abcd15437e73ff4f44345a4a5d1fcaf42' };
const primary = '4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0';
const alternate = '336f319763657d6b0e65a5b5876719e8c8dcdcf9396852be71ee26b73368b29b';
const profileName = 'FixtureProfile';
const interval = 18_000;
const now = () => performance.now();
const hash = bytes => createHash('sha256').update(bytes).digest('hex');

function inputs(rowsPath, indexPath, root) {
  const rowsFile = readFileSync(rowsPath), indexFile = readFileSync(indexPath);
  for (const [bytes, expected] of [[rowsFile,R],[indexFile,I]]) {
    assert.equal(bytes.length, expected.bytes); assert.equal(hash(bytes), expected.sha);
  }
  const rows = JSON.parse(rowsFile), index = JSON.parse(indexFile);
  assert(Array.isArray(rows) && rows.length > 100_000 && rows.length === index.records.length);
  for (const [n, row] of rows.entries()) {
    assert(row[0] !== root && index.records[n].i === n && index.records[n].$?.['1']?.v === row[0], 'Frozen index/row mismatch');
  }
  assert.equal(rowsFile.at(-1), 93);
  assert.equal(indexFile.subarray(-2).toString(), ']}');
  const suffix = (start, added) => createHash('sha256').update(start).update(added).digest('hex');
  const record = {i: rows.length, $: {'0': {v: profileName, n:1}, '1': {v:root,n:1}}};
  return { count: rows.length, expectedRows: suffix(rowsFile.subarray(0,-1), ','+JSON.stringify([root,profileName])+']'),
    expectedIndex: suffix(indexFile.subarray(0,-2), ','+JSON.stringify(record)+']}') };
}

function activity() {
  const a = { graph: null, profile: null, profileEose: null, capture: null, startup:'startup-graph',
    submitted: null, lastGraph:0, lastProfile:0, lastSigned:0, graphCount:0, profileCount:0, signedCount:0,
    partials:0, captures:0, quiet:0, longestQuiet:0, noOp:false, failed:false, timings: [], writes: [] };
  a.observe = line => {
    const t = now();
    if (line.includes('imported graph snapshot')) a.startup = 'startup-profiles';
    const loaded = /loaded graph with (\d+) users and (\d+) profiles/.exec(line);
    if (loaded) { a.startup=''; a.loadedUsers=Number(loaded[1]); a.loadedProfiles=Number(loaded[2]); }
    if (line.includes('persisted PARTIAL graph crawl through distance')) a.partials++;
    if (line.includes('captured named graph exports')) { a.captures++; a.capture = t; }
    if (line.includes('sync cycle finished:')) {
      if (!line.includes('graph_committed=true')) a.failed=true;
      a.graphCount++; a.lastGraph=t;
      a.timings.push({kind:'graph', cycle:a.graphCount, graphRequestToCommitMs:a.graph===null ? null : t-a.graph,
        graphFilesCapturedToProfileSubmissionMs:a.capture===null ? null : t-a.capture});
      if (a.capture!==null) a.submitted=t;
      a.capture=null; a.graph=null;
    }
    if (line.includes('profile cycle finished:')) {
      if (!line.includes('profiles_committed=true')) a.failed=true;
      a.profileCount++; a.lastProfile=t;
      a.timings.push({kind:'profile',cycle:a.profileCount, lastProfileEoseToSavedMs:a.profileEose===null?null:t-a.profileEose});
      a.profile=null; a.profileEose=null;
    }
    if (line.includes('published named social graph snapshot')) {
      a.signedCount++; a.lastSigned=t;
      a.timings.push({kind:'publication',cycle:a.signedCount, submittedToSignedMs:a.submitted===null?null:t-a.submitted});
      a.submitted=null;
    }
  };
  a.labels = (t, mem) => {
    const flags = [];
    if (a.startup) flags.push(a.startup);
    if (a.graph!==null) flags.push('graph-crawl');
    if (a.profile!==null) flags.push(a.profileEose===null ? 'profile-fetch':'profile-response-and-durable-save');
    if (a.capture!==null) flags.push('graph-checkpoint-and-profile-capture');
    if (a.submitted!==null || mem.helpers>0 && !a.startup) flags.push('publication-or-helper');
    // Both completed loops sleep for this interval. Stop early enough to exclude
    // pre-request work at the start of the next cycle, even with buffered logs.
    const idle = !flags.length && a.lastGraph && a.lastProfile && a.lastSigned>=a.lastGraph &&
      t < Math.min(a.lastGraph,a.lastProfile) + interval - 1000 && mem.helpers===0;
    if (idle) {
      if (!a.quiet) a.quiet=t;
      a.longestQuiet=Math.max(a.longestQuiet,t-a.quiet); flags.push('idle');
    } else a.quiet=0;
    return flags;
  };
  return a;
}

export async function runProfileMemory(ctx) {
  const {root,key,publishKey,stableFriend,naming,storage,upstream,seed,scratch,output} = ctx;
  assert.equal(process.platform,'linux');
  assert.deepEqual(readFileSync('/proc/net/dev','utf8').split('\n').filter(s=>s.includes(':')).map(s=>s.split(':')[0].trim()),['lo']);
  assert(!process.env.MALLOC_ARENA_MAX, 'Use the qualified default allocator');
  assert.equal(statSync(seed).size,G.bytes); assert.equal(await ctx.hashFile(seed),G.sha);
  const corpus=inputs(ctx.profileRows,ctx.profileIndex,root);
  const handle=ctx.testKey('new-list-owner'), author=ctx.getPublicKey(handle), target=ctx.getPublicKey(ctx.testKey('new-target'));
  const event=(kind,sk,targets,content='')=>{
    const e=ctx.finalizeEvent({kind,created_at:1690000000+Number(kind===0),tags:targets.map(x=>['p',x]),content},sk);
    assert(ctx.verifyEvent(e)); return e;
  };
  const initial=[event(3,key,[primary,alternate,stableFriend,author]),event(10000,key,[stableFriend])];
  const changed=[event(3,handle,[stableFriend,target]),event(10000,handle,[stableFriend])];
  const profile=event(0,key,[],JSON.stringify({name:profileName}));
  const a=activity(), directory=path.join(scratch,'representative-profiles');
  const home=ctx.envFor(directory,naming,storage,publishKey);
  const rowsOnDisk=path.join(directory,'profileData.large.json'), indexOnDisk=path.join(directory,'profileIndex.json');
  assert(!existsSync(indexOnDisk)); copyFileSync(ctx.profileRows,rowsOnDisk);
  const identities=new Map(); let origin=now();
  const watcher=watch(directory,(_event,name)=>{
    if (!['profileData.large.json','profileIndex.json'].includes(name)) return;
    const st=statSync(path.join(directory,name),{bigint:true,throwIfNoEntry:false});
    if (!st) return;
    const id=String(st.ino)+'-'+String(st.mtimeNs);
    if (identities.get(name)===id) return;
    identities.set(name,id);
    a.writes.push({file:name==='profileIndex.json'?'index':'rows',sinceStartMs:now()-origin,profileCycle:a.profileCount+1});
  });
  ctx.setEvents(initial);
  let failure, worker, sampler, firstSigned;
  upstream.onRequest=id=>{
    try {
      const t=now();
      if (String(id).startsWith('graph-crawl-')) a.graph??=t;
      if (!String(id).startsWith('profile-crawl-')) return;
      a.profile??=t; a.profileEose=null;
      if (!a.noOp && a.profileCount >= 1) {
        // The first sweep completed with zero profile events and persisted both
        // files. This missing synthetic profile is eligible for unbounded history
        // again on the second sweep, when the relay first makes its event available.
        assert.equal(a.loadedProfiles,corpus.count);
        assert.equal(hash(readFileSync(rowsOnDisk)),R.sha); assert.equal(hash(readFileSync(indexOnDisk)),I.sha);
        a.noOp=true; ctx.setEvents([...initial,profile]);
      }
    } catch(e) { failure??=e; }
  };
  upstream.afterEose=id=>{ if(String(id).startsWith('profile-crawl-')) a.profileEose=now(); };
  const ok=()=>{ worker.check(); sampler.check(); if(failure)throw failure; assert(!a.failed,'Crawl or profile persistence failed'); };
  const deadline=now()+15*60_000;
  try {
    origin=now();
    worker=await ctx.launch(directory,{...home,LEGACY_SOCIAL_GRAPH_BINARY_PATH:seed},upstream,storage,true,{
      env:{CRAWL_DISTANCE:'1',PROFILE_CRAWL_DISTANCE:'0',SYNC_INTERVAL_MS:String(interval),GRAPH_PUBLISH_INTERVAL_SECS:'1'},
      onLine:a.observe});
    ctx.track(worker);
    sampler=memorySampler(worker.pid,a.labels,process.env.PROFILE_MEMORY_CGROUP);
    while (true) {
      ok();
      if (!firstSigned && a.graphCount>=2 && ctx.names.length && a.noOp) {
        firstSigned=ctx.names[0]; ctx.setEvents([...initial,profile,...changed]);
      }
      if (a.graphCount===5 && a.profileCount>=5 && a.signedCount>=5 && a.longestQuiet>=3000) break;
      assert(a.graphCount<=5,'Fifth cycle did not meet the fixed comparison window');
      assert(now()<deadline,'Profile memory run timed out');
      await sleep(75);
    }
    assert(a.noOp && firstSigned && a.partials===10 && a.captures===5);
    const end=now();
    const report=sampler.stop(); sampler=null;
    assert(report.phases.idle.samples>=20 && report.phases.all.peak.groupPss>0);
    await worker.stop(); ctx.track(undefined); worker=null;
    const diskRows=await ctx.hashFile(rowsOnDisk), diskIndex=await ctx.hashFile(indexOnDisk);
    assert.equal(diskRows,corpus.expectedRows); assert.equal(diskIndex,corpus.expectedIndex);
    const readStart=now();
    const tree=await ctx.readNamed(scratch,'fresh-profile-reader',naming,storage,true);
    const coldReadMs=now()-readStart;
    ctx.checkManifest(tree.files);
    assert.equal(hash(tree.files['profile-data.json']),diskRows);
    assert.equal(hash(tree.files['profile-index.json']),diskIndex);
    for (const e of [...initial,...changed]) assert(tree.files['social-graph-v4.bin'].includes(Buffer.from(e.id,'hex')));
    const checkEnv={...home,DATA_DIR:directory,SOCIAL_GRAPH_ROOT:root,SOCIAL_GRAPH_DB_DIR:path.join(directory,'graph')};
    const cp=JSON.parse((await ctx.run(ctx.binary,['--check-checkpoint'],{env:checkEnv,timeout:300000,maxBuffer:1024**2})).stdout);
    assert(cp.verified && cp.users===352918 && cp.follows===16996364 && cp.mutes===142411 && cp.follow_revisions===61478 && cp.mute_revisions===10862);
    const checkpoint={users:cp.users,follows:cp.follows,mutes:cp.mutes,followRevisions:cp.follow_revisions,muteRevisions:cp.mute_revisions};
    const fileHashes={};
    for (const [name, bytes] of Object.entries(tree.files)) if (name!=='manifest.json') fileHashes[name.startsWith('mobile-')?'alternate-mobile.bin':name]=hash(bytes);
    assert(Object.keys(fileHashes).length===7 && upstream.writes===0 && naming.errors.length===0);
    mkdirSync(output);
    writeFileSync(path.join(output,'receipt.json'),JSON.stringify({passed:true,sourceRows:corpus.count,
      finalRows:corpus.count+1,seedBytes:G.bytes,binarySha256:await ctx.hashFile(ctx.binary),
      inputDigests:{graph:G.sha,profiles:R.sha,profileIndex:I.sha},inputBytes:{graph:G.bytes,profiles:R.bytes,profileIndex:I.bytes},
      startupUsers:a.loadedUsers,startupProfiles:a.loadedProfiles,noOpProfileBytesVerified:true,uniformAddedProfileVerified:true,freshSignedRead:true,cycles:a.graphCount,profileCycles:a.profileCount,
      distanceCheckpoints:a.partials,publications:a.signedCount,fixtureRunMs:end-origin,coldReadMs,verifiedIdleMs:a.longestQuiet,
      timings:a.timings,atomicWrites:a.writes,checkpoint,fileHashes,phases:report.phases,
      sampling:{intervalMs:report.intervalMs,maxGapMs:report.maxGapMs,pid:report.pid,cgroupMeasured:report.cgroupMeasured,
        missedHelpers:report.missedHelpers,unmeasurableHelperPss:report.unmeasurableHelperPss}},null,2)+'\n',{flag:'wx'});
  } catch (error) {
    if (!existsSync(output)) mkdirSync(output);
    if (!existsSync(path.join(output,'receipt.json'))) writeFileSync(path.join(output,'receipt.json'), JSON.stringify({
      passed:false, sourceRows:corpus.count, cycles:a.graphCount, profileCycles:a.profileCount, publications:a.signedCount,
      noOpVerified:a.noOp, longestQuietMs:a.longestQuiet, distanceCheckpoints:a.partials, timings:a.timings, atomicWrites:a.writes,
    },null,2)+'\n',{flag:'wx'});
    throw error;
  } finally {
    watcher.close(); upstream.onRequest=undefined; upstream.afterEose=undefined;
    if(sampler) { try {sampler.stop();} catch {} }
    if(worker) { await worker.stop(); ctx.track(undefined); }
  }
}
