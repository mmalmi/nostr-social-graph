/** Real cap-100 relay, signed graph heads, durable checkpoint and forwarding.
 * Run against a prebuilt candidate, or --expect-old-truncation against dc81464.
 * Never compiles, inherits credentials, or contacts a public relay.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync, statfsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const args=process.argv.slice(2),argument=args.shift();assert(argument&&!argument.startsWith('--'),'Pass an existing server executable');const binary=path.resolve(argument);assert(existsSync(binary),'Server executable must exist');
const option=(name,fallback)=>{const i=args.indexOf(name);return i<0?fallback:args[i+1]};
const output=path.resolve(option('--output','crawl-cap-evidence'));assert(!existsSync(output),'Keep prior evidence');
const here=path.dirname(fileURLToPath(import.meta.url)),require=createRequire(path.resolve(option('--dependencies',path.join(here,'../../../../ts/package.json'))));
const {WebSocketServer,WebSocket}=require('ws'),{finalizeEvent,getPublicKey,verifyEvent}=require('nostr-tools');
const sha=b=>createHash('sha256').update(b).digest('hex'),old=args.includes('--expect-old-truncation'),recovery=args.includes('--recovery');assert(!(old&&recovery));
const owned=new Set();const overallDeadline=setTimeout(()=>{for(const child of owned){try{process.kill(-child.pid,'SIGKILL')}catch{}}throw Error('fixture deadline')},recovery?150000:90000);
mkdirSync(output,{recursive:true});const disk=statfsSync(output);assert(disk.bavail*disk.bsize>=20*1024**3,'20 GiB fixture reserve');
const keys=Array.from({length:128},(_,i)=>{const key=new Uint8Array(32);key[31]=i+1;return key});
const authors=keys.map(getPublicKey),root=authors[0],at=Math.floor(Date.now()/1000)-10;
// Mute only the root (which is exempt from overmute admission); every author
// under the batch-cap test must remain eligible for both signed list heads.
const events=keys.flatMap((key,i)=>[3,10000].map(kind=>finalizeEvent({kind,created_at:at,content:'',tags:(kind===3?(i===0?authors.slice(1):[root]):[root]).map(pubkey=>['p',pubkey])},key)));
const matches=(event,filter)=>(!filter.kinds||filter.kinds.includes(event.kind))&&(!filter.authors||filter.authors.includes(event.pubkey))&&(!filter.ids||filter.ids.includes(event.id))&&(filter.since===undefined||event.created_at>=filter.since);
const source=new WebSocketServer({host:'127.0.0.1',port:0});await once(source,'listening');
const sink=new WebSocketServer({host:'127.0.0.1',port:0});await once(sink,'listening');
const requests=[],liveSubscriptions=new Map(),forwarded=new Set(),errors=[];let uploadsToSource=0,outage=false,holdProfiles=false;const profilePending=new Set(),profileClosed=new Set(),forwardAttempts=[];
source.on('connection',socket=>{const live=new Map();liveSubscriptions.set(socket,live);socket.on('close',()=>liveSubscriptions.delete(socket));socket.on('message',raw=>{
 try{const frame=JSON.parse(raw.toString());if(frame[0]==='EVENT'){uploadsToSource++;return}if(frame[0]==='CLOSE'){live.delete(frame[1]);if(profilePending.delete(frame[1]))profileClosed.add(frame[1]);return}if(frame[0]!=='REQ')return;
  const [_,id,...filters]=frame;requests.push({id,filters,at:Date.now()});if(holdProfiles&&filters.some(f=>f.kinds?.includes(0))){profilePending.add(id);return;}
  // Historical bodies only enter via the production crawl. Live subscriptions
  // receive EOSE only, so they cannot hide a truncated historical request.
  if(String(id).startsWith('graph-crawl-')||id==='old-cap-control'){
   const selected=new Map();for(const filter of filters)for(const event of events.filter(event=>matches(event,filter)).slice(0,Math.min(filter.limit??100,100)))selected.set(event.id,event);
   for(const event of selected.values())socket.send(JSON.stringify(['EVENT',id,event]));
  }else live.set(id,filters);
  socket.send(JSON.stringify(['EOSE',id]));
 }catch(error){errors.push(String(error));socket.close()}
})});
sink.on('connection',socket=>socket.on('message',raw=>{try{const frame=JSON.parse(raw.toString());if(frame[0]==='EVENT'){assert(verifyEvent(frame[1]));forwardAttempts.push({id:frame[1].id,at:Date.now(),outage});if(outage){socket.terminate();return;}forwarded.add(frame[1].id);socket.send(JSON.stringify(['OK',frame[1].id,true,'fixture accepted']))}}catch(error){errors.push(String(error));socket.close()}}));
const upstream=`ws://127.0.0.1:${source.address().port}`,forwardUrl=`ws://127.0.0.1:${sink.address().port}`;
const probe=new WebSocket(upstream);await once(probe,'open');let capped=0;const eose=new Promise((resolve,reject)=>{probe.on('error',reject);probe.on('message',raw=>{const f=JSON.parse(raw.toString());if(f[0]==='EVENT')capped++;if(f[0]==='EOSE')resolve()})});probe.send(JSON.stringify(['REQ','old-cap-control',{authors,kinds:[3,10000]}]));await eose;probe.close();assert.equal(capped,100,'Old128-author/both-kind filter must reproduce the relay cap');
const allocator=createServer().listen(0,'127.0.0.1');await once(allocator,'listening');const port=allocator.address().port;await new Promise(resolve=>allocator.close(resolve));
const directory=path.join(output,'data');mkdirSync(directory);const env={PATH:process.env.PATH,HOME:path.join(directory,'home'),TMPDIR:path.join(directory,'tmp'),XDG_CONFIG_HOME:path.join(directory,'config')};for(const key of ['HOME','TMPDIR','XDG_CONFIG_HOME'])mkdirSync(env[key]);
Object.assign(env,{BIND_ADDRESS:'127.0.0.1',PORT:String(port),DATA_DIR:directory,SOCIAL_GRAPH_DB_DIR:path.join(directory,'graph'),SOCIAL_GRAPH_ROOT:root,LEGACY_SOCIAL_GRAPH_BINARY_PATH:path.join(directory,'absent-legacy.bin'),RELAY_URLS:upstream,CRAWL_DISTANCE:'1',CRAWL_BATCH_SIZE:'128',PROFILE_CRAWL_DISTANCE:'0',PROFILE_CRAWL_LIMIT:'1',SYNC_INTERVAL_MS:'1000',GRAPH_FORWARD_RELAY_URL:forwardUrl,GRAPH_FORWARD_ALLOWLIST_PATH:path.join(directory,'allowlist'),RUST_LOG:'info'});
const log=createWriteStream(path.join(output,'server.log'),{flags:'wx'}),child=spawn(binary,[],{env,detached:true,stdio:['ignore','pipe','pipe']});child.stdout.pipe(log,{end:false});child.stderr.pipe(log,{end:false});const closed=new Promise(resolve=>child.once('close',resolve));
owned.add(child);let stopping=false,healthSeen=false;const healthSamples=[];let recoveryEvidence=null;
async function health(){const start=performance.now();try{const response=await fetch(`http://127.0.0.1:${port}/healthz`,{signal:AbortSignal.timeout(1000)});const value=await response.json();healthSeen=true;healthSamples.push({milliseconds:performance.now()-start,status:response.status,at:Date.now(),forwardingFailures:value.freshness?.forwarding_failures,forwardRepair:value.freshness?.last_forward_reconciliation_at});return value}catch(error){if(healthSeen)healthSamples.push({milliseconds:performance.now()-start,error:String(error)});return null}}
async function until(check,label,timeout=30000){const end=Date.now()+timeout;while(Date.now()<end){assert.equal(child.exitCode,null,'Server exited');if(await check())return;await new Promise(resolve=>setTimeout(resolve,25))}throw Error(`Timed out: ${label}`)}
function durable(){const file=path.join(directory,'graph','social-graph-root.json');if(!existsSync(file))return null;const meta=JSON.parse(readFileSync(file,'utf8'));const bytes=readFileSync(path.join(directory,'graph','blobs',meta.cid.slice(0,2),meta.cid.slice(2,4),meta.cid.slice(4)));assert.equal(sha(bytes),meta.cid);assert.equal(bytes.length,meta.size);assert.equal(bytes[0],4);return bytes}
const sampler=(async()=>{while(!stopping){await health();await new Promise(resolve=>setTimeout(resolve,25))}})();
try{
 await until(async()=>Boolean((await health())?.healthy),'durable complete graph cycle');const snapshot=durable();assert(snapshot);const present=events.filter(event=>snapshot.includes(Buffer.from(event.id,'hex')));const crawl=requests.filter(r=>String(r.id).startsWith('graph-crawl-'));
 if(old){assert(present.length<256);assert(crawl.some(r=>r.filters.some(f=>f.authors.length>32&&f.limit===undefined)));}
 else{
  assert.equal(present.length,256,'All128 authors must retain both exact signed list IDs');assert(crawl.every(r=>r.filters.every(f=>f.authors.length<=32&&f.limit===f.authors.length*2&&f.limit<=64)));
  await until(()=>events.every(event=>forwarded.has(event.id)),'all historical heads forwarded');
  const live=finalizeEvent({kind:10000,created_at:Math.floor(Date.now()/1000)+1,content:'',tags:[['p',root]]},keys[127]);events.splice(events.findIndex(event=>event.pubkey===authors[127]&&event.kind===10000),1,live);let deliveries=0;
  for(const [socket,subscriptions] of liveSubscriptions)for(const [id,filters] of subscriptions)if(socket.readyState===1&&filters.some(filter=>matches(live,filter))){socket.send(JSON.stringify(['EVENT',id,live]));deliveries++;}
  assert(deliveries>0,'Live monitor remains subscribed after full repair');await until(()=>forwarded.has(live.id),'post-checkpoint live head forwarded');await until(()=>durable()?.includes(Buffer.from(live.id,'hex')),'post-checkpoint live head durable');
  const final=await health();assert(final.healthy);assert.equal(final.freshness.forwarding_failures,0);
 }
 if(recovery){
  // Hold one real profile fetch open. Live allowlist maintenance must run while
  // that request is pending; recovery may stop profiles only after its CLOSE.
  holdProfiles=true;
  await until(()=>profilePending.size>0,'in-flight profile request');
  const heldProfileIds=[...profilePending],before=await health();
  const beforeRepair=before.freshness.last_forward_reconciliation_at;
  const outageStarted=Date.now();outage=true;
  for(const socket of sink.clients)socket.terminate();
  const newKey=new Uint8Array(32);newKey[30]=1;newKey[31]=1;
  const newAuthor=getPublicKey(newKey),created_at=Math.floor(Date.now()/1000)+2;
  const replacement=finalizeEvent({kind:3,created_at,content:'',tags:[...authors.slice(1),newAuthor].map(key=>['p',key])},keys[0]);
  const introduced=[3,10000].map(kind=>finalizeEvent({kind,created_at,content:'',tags:[['p',root]]},newKey));
  events.splice(events.findIndex(event=>event.kind===3&&event.pubkey===root),1,replacement);
  events.push(...introduced);
  function sendLive(event){let delivered=0;for(const [socket,subscriptions] of liveSubscriptions)for(const [id,filters] of subscriptions)if(socket.readyState===1&&filters.some(filter=>matches(event,filter))){socket.send(JSON.stringify(['EVENT',id,event]));delivered++;}assert(delivered>0);}
  sendLive(replacement);
  await until(async()=>(await health())?.users>=129,'new author admitted from live follow list');
  for(const event of introduced)sendLive(event);
  await until(async()=>(await health())?.freshness.forwarding_failures>0,'observable transport loss');
  const failureObserved=Date.now();assert.equal((await health()).healthy,false);
  await until(()=>readFileSync(path.join(directory,'allowlist'),'utf8').split('\n').includes(newAuthor),'coalesced live allowlist update',12000);
  const allowlistObserved=Date.now();
  assert(allowlistObserved-outageStarted<15000,'Allowlist must update before the 30-second repair');
  assert.equal((await health()).healthy,false,'An updated allowlist alone does not repair missed heads');
  outage=false;holdProfiles=false;
  const requestBoundary=requests.length;
  await until(async()=>{
   const status=await health();
   if(status?.healthy){
    assert(status.freshness.last_forward_reconciliation_at>beforeRepair,'Healthy requires a new full forwarding repair');
    assert(events.every(event=>forwarded.has(event.id)),'Healthy requires all currently available heads delivered');
    return true;
   }
   return false;
  },'bounded full forwarding repair after outage',75000);
  const recoveredAt=Date.now(),status=await health(),checkpoint=durable();
  for(const event of [replacement,...introduced])assert(checkpoint.includes(Buffer.from(event.id,'hex')),'Repaired new heads must be durable');
  assert.equal(status.freshness.forwarding_failures,0);
  assert.equal(status.freshness.checkpoint_is_partial,false);
  assert(requests.slice(requestBoundary).some(request=>String(request.id).startsWith('graph-crawl-')&&request.filters.every(filter=>filter.since===undefined)),'Missed forwarding forces full historical repair, not a since-only refresh');
  assert(heldProfileIds.every(id=>profileClosed.has(id)),'Profile subscription cleanup must send CLOSE before repair');
  const prematureHealthy=healthSamples.filter(sample=>sample.at>=failureObserved&&sample.at<recoveredAt&&sample.status===200);
  assert(prematureHealthy.every(sample=>sample.forwardRepair>beforeRepair&&sample.forwardingFailures===0));
  recoveryEvidence={outageStarted,failureObserved,allowlistObserved,recoveredAt,recoveryMs:recoveredAt-failureObserved,heldProfileIds,closedProfileIds:[...profileClosed],newAuthor,newHeadIds:[replacement,...introduced].map(event=>event.id),freshness:status.freshness,forwardAttempts};
 }
 assert.deepEqual(errors,[]);assert.equal(uploadsToSource,0);assert(healthSamples.some(sample=>sample.status===200));
 writeFileSync(path.join(output,'snapshot.bin'),snapshot);writeFileSync(path.join(output,'receipt.json'),JSON.stringify({passed:true,mode:old?'old-negative-control':'candidate',authors:128,expectedHeads:256,durableHeads:present.length,oldFilterCappedHeads:capped,crawlRequests:crawl,recovery:recoveryEvidence,forwardedHeads:forwarded.size,upstreamUploads:uploadsToSource,healthSamples,limits:'Health/forward continuity is measured on the256-head protocol fixture; production-scale pruning latency is a separate ignored Rust benchmark. No production-host latency claim.',binarySha256:sha(readFileSync(binary))},null,2)+'\n');console.log(JSON.stringify({passed:true,mode:old?'old-negative-control':'candidate',durableHeads:present.length,forwardedHeads:forwarded.size,maxHealthMs:Math.max(...healthSamples.map(s=>s.milliseconds))}));
}finally{
 stopping=true;if(child.exitCode===null){try{process.kill(-child.pid,'SIGTERM')}catch{}}const kill=setTimeout(()=>{try{process.kill(-child.pid,'SIGKILL')}catch{}},3000);await closed;owned.delete(child);clearTimeout(kill);await sampler;await new Promise(resolve=>log.end(resolve));for(const server of [source,sink]){for(const socket of server.clients)socket.terminate();await new Promise(resolve=>server.close(resolve))}clearTimeout(overallDeadline);
}
