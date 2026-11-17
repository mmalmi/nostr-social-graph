import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { monitor, save } from './memory_monitor.mjs';
const seeds = {
  d9d11e34916b33fb8540f3a124f5968c0d67db7c45d75412a17c382f011256ca: { tier: 'mobile', bytes: 1416341, cycles: 8, minUsers: 20000, minFollows: 150000, ceiling: 3 * 2**30 },
  aeae583ae888f7fec2052f678ff30ae2b031ed4e8d77d0fe0c644e35c2bd6fc0: { tier: 'full', bytes: 55750905, cycles: 5, minUsers: 300000, minFollows: 15000000, ceiling: 12 * 2**30 },
};
const primary = '4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0';
const alternate = '336f319763657d6b0e65a5b5876719e8c8dcdcf9396852be71ee26b73368b29b';
const alternateMember = 'mobile-' + alternate + '.bin';
const mib = 2**20;
async function digestResponse(origin, member, maxSize) {
  const response = await fetch(origin + member, { signal: AbortSignal.timeout(150000) });
  assert.equal(response.status, 200, member);
  let length=0; const hash=createHash('sha256');
  for await (const b of response.body) { length+=b.length; assert(length<=maxSize, member + ' exceeds limit'); hash.update(b); }
  return {size:length,sha256:hash.digest('hex')};
}
async function wait(check, limit, label) {
  const start = Date.now(); while (Date.now()-start < limit) { const result = await check(); if (result) return result; await sleep(200); }
  throw Error('Timed out waiting for ' + label);
}
export async function runMemoryCycles(ctx) {
  const { binary, seed, scratch, output, root, key, publishKey, stableFriend, naming, storage, upstream } = ctx;
  assert.equal(process.platform,'linux'); assert.deepEqual(readFileSync('/proc/net/dev','utf8').split('\n').filter(row=>row.includes(':')).map(row=>row.split(':')[0].trim()), ['lo'], 'Use the existing isolated Linux fixture namespace');
  const seedHash=await ctx.hashFile(seed), profile=seeds[seedHash]; assert(profile, 'Unknown memory seed');
  assert.equal(statSync(seed).size,profile.bytes); mkdirSync(output);
  const handle=ctx.testKey('new-list-owner'), author=ctx.getPublicKey(handle), target=ctx.getPublicKey(ctx.testKey('new-target'));
  const stamp=1690000000;
  function event(kind, sk, tags, content='') { const e=ctx.finalizeEvent({kind,created_at:stamp+(kind===0?1:0),tags:tags.map(pk=>['p',pk]),content},sk); assert(ctx.verifyEvent(e)); return e; }
  // A never-crawled author is requested without a since cursor. Its signed historical lists can
  // therefore arrive on a later real crawl while baseline and candidate use identical event IDs.
  const base=[event(3,key,[primary,alternate,stableFriend,author]),event(10000,key,[stableFriend]),event(0,key,[],JSON.stringify({name:'Memory fixture profile'}))];
  const extra=[event(3,handle,[stableFriend,target]),event(10000,handle,[stableFriend])];
  ctx.setEvents(base); const directory=path.join(scratch,'measured');
  const home=ctx.envFor(directory,naming,storage,publishKey), env={ ...home, LEGACY_SOCIAL_GRAPH_BINARY_PATH:seed };
  const timing={ CRAWL_DISTANCE:'1', PROFILE_CRAWL_DISTANCE:'0', SYNC_INTERVAL_MS:'3500', GRAPH_PUBLISH_INTERVAL_SECS:'1' };
  let cycles=0, partials=0, exports=0, totalWrites=0, changedAt=null, changedNameCount=null, firstSigned=null;
  let worker, sampler; const tests=[], failed=[];
  function observe(line) {
    if (line.includes('persisted PARTIAL graph crawl through distance')) partials++;
    if (line.includes('captured named graph exports')) exports++;
    if (line.includes('published named social graph snapshot')) totalWrites++;
    if (line.includes('sync cycle finished:')) {
      if (!line.includes('graph_committed=true profiles_committed=true')) failed.push(line.slice(-400));
      else { cycles++; sampler?.checkpoint(cycles); sampler?.mark('cycle-'+(cycles+1)); }
    }
  }
  const trial = { version:1, tier:profile.tier, seedSha256:seedHash, seedBytes:profile.bytes, binarySha256:await ctx.hashFile(binary),
    primaryRoot:root, fixtureEventIds:[...base,...extra].map(e=>e.id), desiredCycles:profile.cycles };
  save(path.join(output,'trial.json'),trial);
  try {
    worker=await ctx.launch(directory,env,upstream,storage,true,{env:timing,onLine:observe});ctx.track(worker);
    sampler=monitor(worker.pid,output,'crawler',profile.ceiling);
    await wait(()=>{
      worker.check();sampler.check();assert.deepEqual(failed,[]);assert.deepEqual(naming.errors,[]);
      if (cycles>=2 && changedAt===null && ctx.names.length) {
        firstSigned=ctx.names[0];changedAt=cycles;changedNameCount=ctx.names.length;ctx.setEvents([...base,...extra]);
      }
      return changedAt!==null && cycles>=Math.max(profile.cycles,changedAt+3) && ctx.names.length>changedNameCount && totalWrites>=2;
    },profile.tier==='full'?25*60*1000:8*60*1000,'changed and unchanged durable crawl cycles');
    const measured=sampler.finish();sampler=null; assert(measured.settled.length>=profile.cycles);
    assert(partials>=2*cycles,'Both crawl distances must finish; if production changes diagnostic text, verify its actual checkpoint operations.');
    assert(exports>=2);const health=await wait(async()=>{worker.check();const value=await worker.health();return value?.healthy&&value;},120000,'healthy completed graph');
    assert(health.users>=profile.minUsers && health.follows>=profile.minFollows);  await worker.stop();worker=null;ctx.track(undefined);
    const final=await ctx.readNamed(scratch,'cold-final',naming,storage,true); const manifest=ctx.checkManifest(final.files);
    for (const filename of ['social-graph-v4.bin','mobile.bin']) {
      for (const e of [...base,...extra].filter(e=>e.kind!==0)) assert(final.files[filename].includes(Buffer.from(e.id,'hex')),filename+' missing revision '+e.id);
    }
    const allow=final.files['allowlist.txt'].toString().split('\n');for(const pk of [root,author,stableFriend,target])assert(allow.includes(pk));
    const rows=JSON.parse(final.files['profile-data.json']);assert(rows.some(r=>r[0]===root&&r[1]==='Memory fixture profile'));
    const firstCid=firstSigned.tags.find(t=>t[0]==='hash')[1]+':'+firstSigned.tags.find(t=>t[0]==='key')[1];assert.notEqual(firstCid,final.cid);
    const cold=ctx.envFor(path.join(scratch,'first-cold'),naming,storage);const old=await ctx.run(ctx.htree,['--data-dir',cold.HTREE_DATA_DIR,'cat',firstCid+'/manifest.json'],{env:cold,timeout:120000,maxBuffer:mib});
    const first=JSON.parse(old.stdout);assert.notEqual(first.files['social-graph-v4.bin'].sha256,manifest.files['social-graph-v4.bin'].sha256,'changed signed graph must produce changed graph data');
    const checkEnv={...home,DATA_DIR:directory,SOCIAL_GRAPH_ROOT:root,SOCIAL_GRAPH_DB_DIR:path.join(directory,'graph')};const checkpoint=JSON.parse((await ctx.run(binary,['--check-checkpoint'],{env:checkEnv,timeout:300000,maxBuffer:mib})).stdout);
    assert(checkpoint.verified&&checkpoint.users>=profile.minUsers&&checkpoint.follows>=profile.minFollows);assert(checkpoint.mutes>0&&checkpoint.follow_revisions>0&&checkpoint.mute_revisions>0);
    // The source serves no events during cold startup. Disk restoration supplies the client data.
    ctx.setEvents([]);worker=await ctx.launch(directory,home,upstream,storage,false,{env:{...timing,SYNC_INTERVAL_MS:'120000'}});ctx.track(worker);
    sampler=monitor(worker.pid,output,'restart',profile.ceiling);
    const restored=await wait(async()=>{worker.check();sampler.check();const h=await worker.health();return h?.users>=profile.minUsers&&h;},300000,'cold native graph restore');
    sampler.mark('restart-client-reads');assert.equal(restored.users,checkpoint.users);
    for (const [route,file,bound] of [['/social-graph','social-graph.bin',160*mib],['/social-graph/mobile','mobile.bin',4*mib],['/social-graph/mobile?root='+alternate,alternateMember,4*mib],['/allowlist','allowlist.txt',64*mib]]) {
      const actual=await digestResponse(worker.origin,route,bound),expected=Object.hasOwn(manifest.files,file)?manifest.files[file]:manifest.mobile_roots[alternate];
      assert.equal(actual.sha256,expected.sha256,file+' readback after restart');assert.equal(actual.size,expected.size);tests.push(file);
    }
    sampler.finish();sampler=null;await worker.stop();worker=null;ctx.track(undefined);
    assert.equal(upstream.writes,0);assert.deepEqual(naming.errors,[]);
    const fileHashes=Object.fromEntries(Object.entries(final.files).filter(([n])=>n!=='manifest.json').map(([n,b])=>[n,ctx.digest(b)]));
    save(path.join(output,'receipt.json'),{...trial,passed:true,cycles,partials,exports,totalWrites,changedAt,checkpoint,fileHashes,
      checks:tests,coldNamedRead:true,oldImmutableRead:true,signedPublisher:ctx.getPublicKey(publishKey),firstEventId:firstSigned.id,upstreamWrites:upstream.writes});
  } finally { if (sampler) sampler.finish(); }
}
