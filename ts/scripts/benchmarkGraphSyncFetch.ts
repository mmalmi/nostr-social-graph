// Measured application bytes over loopback WebSocket, not mobile CPU or WAN latency.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {WebSocket} from 'ws';
import {SocialGraph} from '../src/SocialGraph';
import {syncGraphLists} from '../src/publicGraphSync';
import {graphSyncRelay,signedGraphLists} from '../tests/helpers/graphSyncRelay';

const {authors,events}=signedGraphLists();
const versions:{label:string;sync:typeof syncGraphLists}[]=[];
// Supply the verified previous release's dist/publicGraphSync.js for a comparison.
if(process.argv[2])versions.push({label:'previous-release',sync:(await import(pathToFileURL(path.resolve(process.argv[2])).href)).syncGraphLists});
// An optional second path qualifies the packed current helper using the same fixture.
versions.push({label:'current',sync:process.argv[3]?(await import(pathToFileURL(path.resolve(process.argv[3])).href)).syncGraphLists:syncGraphLists});
const rows=[];
for(const {label,sync} of versions)for(const cap of [Infinity,100]){
 const relay=await graphSyncRelay(events,cap),graph=new SocialGraph(authors[0]);
 try{
  const stats=await sync({graph,authors,relays:[relay.url],webSocket:WebSocket as unknown as typeof globalThis.WebSocket});
  await relay.settled();assert.deepEqual(relay.errors,[]);
  assert.equal(stats.sentBytes,relay.wire.sentBytes);assert.equal(stats.receivedBytes,relay.wire.receivedBytes);
  const requests=relay.requests.filter(frame=>frame[0]==='REQ') as [string,string,{ids:string[]}][];
  const wire={...relay.wire};
  const row={label,relayEventCap:Number.isFinite(cap)?cap:null,stats,requestCount:requests.length,maxIdsPerRequest:Math.max(0,...requests.map(frame=>frame[2].ids.length)),wire,totalApplicationBytes:wire.sentBytes+wire.receivedBytes};
  if(stats.completedAuthors===authors.length){
   const warm=await sync({graph,authors,relays:[relay.url],webSocket:WebSocket as unknown as typeof globalThis.WebSocket});
   assert.equal(warm.receivedEvents,0);assert.equal(warm.failedRelays,0);Object.assign(row,{warm});
  }
  rows.push(row);
 }finally{await relay.close();}
}
console.log(JSON.stringify({note:'Same 130 synthetic authors / 260 signed follow and mute events in every run. Includes JSON frames, NEG hex, author filters, exact-ID REQ, EVENT envelopes, CLOSE and EOSE. eventPayloadBytes counts signed event JSON alone; remaining bytes are protocol overhead. Excludes WebSocket framing, TCP/IP and TLS; elapsed times are diagnostic loopback measurements, not mobile CPU or network performance.',fixtureSha256:createHash('sha256').update(JSON.stringify(events)).digest('hex'),authors:authors.length,events:events.length,rows},null,2));
