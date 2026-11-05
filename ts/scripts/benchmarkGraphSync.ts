// Deterministic protocol overhead, not a mobile CPU or live-relay latency claim.
import {createHash} from 'node:crypto';
import {gzipSync} from 'node:zlib';
import {Negentropy,NegentropyStorageVector} from '../src/vendor/negentropy.js';
const id=(s:string)=>createHash('sha256').update(s).digest('hex');
const rows=[];
for(const changed of [0,30,300,3000]){
 const started=performance.now();let wire=0,rounds=0,downloadIds=0;
 // 30,000 authors, two revisions each. Author filters are counted on wire.
 for(let offset=0;offset<30000;offset+=256){
  const a=new NegentropyStorageVector(),b=new NegentropyStorageVector(),authors=[];
  for(let i=offset;i<Math.min(offset+256,30000);i++){
   authors.push(id(`author:${i}`));
   for(const kind of [3,10000]){a.insert(1700000000+i,id(`${kind}:${i}`));b.insert(1700000000+i+(i<changed?1:0),id(`${kind}:${i}${i<changed?':new':''}`));}
  }
  a.seal();b.seal();const client=new Negentropy(a,16384),server=new Negentropy(b,16384);
  let message:string|null=await client.initiate();wire+=Buffer.byteLength(JSON.stringify(['NEG-OPEN','g',{kinds:[3,10000],authors},message]));
  const needIds=new Set<string>();
  while(message!==null){
   const [response]=await server.reconcile(message);wire+=Buffer.byteLength(JSON.stringify(['NEG-MSG','g',response]));
   const [next,,need]=await client.reconcile(response!);for(const value of need)needIds.add(value);message=next;rounds++;
   if(message!==null)wire+=Buffer.byteLength(JSON.stringify(['NEG-MSG','g',message]));
  }
  wire+=Buffer.byteLength(JSON.stringify(['NEG-CLOSE','g']));
  const needs=[...needIds];downloadIds+=needs.length;
  for(let i=0;i<needs.length;i+=64)wire+=Buffer.byteLength(JSON.stringify(['REQ','g',{ids:needs.slice(i,i+64)}]));
 }
 const example=JSON.stringify({kind:3,pubkey:id('author'),created_at:1700000000,id:id('event'),sig:'a'.repeat(128),content:'',tags:Array.from({length:100},(_,i)=>['p',id(`follows:${i}`)])});
 rows.push({authors:30000,revisionCount:60000,changedAuthors:changed,downloadIds,reconciliationAndRequestBytes:wire,roundTrips:rounds,cpuWallMs:Math.round(performance.now()-started),illustrativeChangedEventBytes:downloadIds*Buffer.byteLength(example),illustrativeEventGzipBytes:gzipSync(example).length});
}
console.log(JSON.stringify({note:'Local reference-protocol measurement. Counts NIP77 JSON/hex framing and author filters, excludes TLS/WebSocket framing. Both kinds change for selected authors. Event payload cost is illustrative at 100 public tags/event, not observed relay data.',rows},null,2));
