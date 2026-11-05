import {describe,it,expect,vi} from 'vitest';
import {WebSocket,WebSocketServer} from 'ws';
import {finalizeEvent,getPublicKey} from 'nostr-tools';
import {SocialGraph} from '../src/SocialGraph';
import {syncGraphLists,startGraphSync,loadGraphBootstrap} from '../src/publicGraphSync';
import {Negentropy,NegentropyStorageVector} from '../src/vendor/negentropy.js';
const secret=new Uint8Array(32).fill(7),pubkey=getPublicKey(secret);
const event=(kind:number,time:number,targets:string[]=[])=>finalizeEvent({kind,created_at:time,tags:targets.map(p=>['p',p]),content:''},secret);
async function relay(events:ReturnType<typeof event>[],unsupported=false,response:{events?:ReturnType<typeof event>[];end?:'EOSE'|'CLOSED'}={}){
 const server=new WebSocketServer({port:0});await new Promise<void>(r=>server.once('listening',r));
 const requests:any[]=[];
 server.on('connection',socket=>{let ne:Negentropy;
 socket.on('message',async raw=>{const m=JSON.parse(raw.toString());requests.push(m);
 if(m[0]==='NEG-OPEN'){
  if(unsupported){socket.send(JSON.stringify(['NEG-ERR',m[1],'unsupported']));return;}
  const storage=new NegentropyStorageVector();for(const e of events)storage.insert(e.created_at,e.id);storage.seal();ne=new Negentropy(storage,16384);
  socket.send(JSON.stringify(['NEG-MSG',m[1],(await ne.reconcile(m[3]))[0]]));
 }else if(m[0]==='NEG-MSG'){socket.send(JSON.stringify(['NEG-MSG',m[1],(await ne.reconcile(m[2]))[0]]));}
 else if(m[0]==='REQ'){for(const e of response.events??events)if(!m[2].ids||m[2].ids.includes(e.id))socket.send(JSON.stringify(['EVENT',m[1],e]));socket.send(JSON.stringify([response.end??'EOSE',m[1]]));}
 });});
 return {url:`ws://127.0.0.1:${(server.address() as {port:number}).port}`,requests,close:()=>new Promise<void>(r=>{for(const socket of server.clients)socket.terminate();server.close(()=>r());})};
}
describe('optional public graph reconciliation',()=>{
 it('fetches only missing signed follow/mute revisions and leaves local newer lists intact',async()=>{
  const follow=event(3,100),mute=event(10000,101,[pubkey]),graph=new SocialGraph(pubkey);graph.handleEvent(follow);
  const server=await relay([follow,mute]);try{
   const stats=await syncGraphLists({graph,authors:[pubkey],relays:[server.url],webSocket:WebSocket as any});
   expect(stats.receivedEvents).toBe(1);expect(stats.appliedEvents).toBe(1);expect(stats.fallbackBatches).toBe(0);
   expect(graph.getListRevision(pubkey,10000)?.id).toBe(mute.id);
   const again=await syncGraphLists({graph,authors:[pubkey],relays:[server.url],webSocket:WebSocket as any});expect(again.receivedEvents).toBe(0);
  }finally{await server.close();}
 });
 it('uses bounded fallback for unsupported relays without rolling back or admitting invalid signatures',async()=>{
  const current=event(3,200),old=event(3,100),invalid={...event(10000,200),sig:'0'.repeat(128)};
  const graph=new SocialGraph(pubkey);graph.handleEvent(current);
  const server=await relay([old,invalid],true);try{
   const stats=await syncGraphLists({graph,authors:[pubkey],relays:[server.url],webSocket:WebSocket as any});
   expect(stats.fallbackBatches).toBe(1);expect(stats.appliedEvents).toBe(0);expect(graph.getListRevision(pubkey,3)?.id).toBe(current.id);expect(graph.getListRevision(pubkey,10000)).toBeUndefined();
  }finally{await server.close();}
 });
 it('does not advertise projected incomplete list IDs',async()=>{
  const full=event(3,100,[getPublicKey(new Uint8Array(32).fill(8))]), graph=new SocialGraph(pubkey);graph.handleEvent(full);graph.markListIncomplete(pubkey,3);
  const server=await relay([full]);try{const stats=await syncGraphLists({graph,authors:[pubkey],relays:[server.url],webSocket:WebSocket as any});expect(stats.receivedEvents).toBe(1);expect(graph.getListRevision(pubkey,3)?.complete).toBe(true);}finally{await server.close();}
 });
 it.each(['missing','invalid'] as const)('does not complete an author window with %s advertised event bodies',async(problem)=>{
  const follow=event(3,100),mute=event(10000,101,[pubkey]),graph=new SocialGraph(pubkey);
  const bodies=problem==='missing'?[follow]:[follow,{...mute,sig:'0'.repeat(128)}];
  const server=await relay([follow,mute],false,{events:bodies});
  try{
   const stats=await syncGraphLists({graph,authors:[pubkey],relays:[server.url],webSocket:WebSocket as any});
   expect(stats.failedRelays).toBe(1);expect(stats.completedAuthors).toBe(0);
   expect(stats.appliedEvents).toBe(1);expect(graph.getListRevision(pubkey,3)?.id).toBe(follow.id);
   expect(graph.getListRevision(pubkey,10000)).toBeUndefined();
  }finally{await server.close();}
 });
 it.each([false,true])('treats CLOSED as failure, including fallback=%s',async(fallback)=>{
  const follow=event(3,100),graph=new SocialGraph(pubkey),server=await relay([follow],fallback,{end:'CLOSED'});
  try{
   const stats=await syncGraphLists({graph,authors:[pubkey],relays:[server.url],webSocket:WebSocket as any});
   expect(stats.failedRelays).toBe(1);expect(stats.completedAuthors).toBe(0);
   expect(graph.getListRevision(pubkey,3)?.id).toBe(follow.id);
  }finally{await server.close();}
 });
 it('counts a verified requested stale body without replacing a newer retained list',async()=>{
  const current=event(3,200),stale=event(3,100),graph=new SocialGraph(pubkey);graph.handleEvent(current);
  const server=await relay([stale]);
  try{
   const stats=await syncGraphLists({graph,authors:[pubkey],relays:[server.url],webSocket:WebSocket as any});
   expect(stats.failedRelays).toBe(0);expect(stats.completedAuthors).toBe(1);expect(stats.appliedEvents).toBe(0);
   expect(graph.getListRevision(pubkey,3)?.id).toBe(current.id);
  }finally{await server.close();}
 });
 it('honors a transfer budget and aborts promptly',async()=>{
  const graph=new SocialGraph(pubkey),server=await relay([event(3,100,Array(100).fill(pubkey))],true);
  try{const stats=await syncGraphLists({graph,authors:[pubkey],relays:[server.url],webSocket:WebSocket as any,maxBytes:1024});expect(stats.limited).toBe(true);expect(stats.appliedEvents).toBe(0);
   const controller=new AbortController();controller.abort();await expect(syncGraphLists({graph,relays:[server.url],signal:controller.signal,webSocket:WebSocket as any})).rejects.toThrow();
  }finally{await server.close();}
 });
});

describe('sync lifecycle and import bounds',()=>{
 it('recovers after graph initialization throws',async()=>{
  vi.useFakeTimers();let calls=0;const errors:unknown[]=[];
  const stop=startGraphSync({getGraph:()=>{calls++;throw new Error('not ready');},relays:[],intervalMs:1000,onError:e=>errors.push(e)});
  try{await vi.advanceTimersByTimeAsync(2100);expect(calls).toBe(3);expect(errors).toHaveLength(3);}finally{stop();vi.useRealTimers();}
 });
 it('has a total deadline even when relay never responds',async()=>{
  const server=new WebSocketServer({port:0});await new Promise<void>(r=>server.once('listening',r));
  try{const before=Date.now();const stats=await syncGraphLists({graph:new SocialGraph(pubkey),authors:[pubkey],relays:[`ws://127.0.0.1:${(server.address() as {port:number}).port}`],webSocket:WebSocket as any,maxDurationMs:100,timeoutMs:5000});expect(Date.now()-before).toBeLessThan(1500);expect(stats.limited).toBe(true);expect(stats.completedAuthors).toBe(0);}finally{for(const socket of server.clients)socket.terminate();await new Promise<void>(r=>server.close(()=>r()));}
 });
 it('rejects an oversized streamed bootstrap before decoding',async()=>{
  vi.stubGlobal('fetch',async()=>new Response(new Uint8Array(2048),{headers:{'x-graph-root':pubkey}}));
  try{await expect(loadGraphBootstrap({root:pubkey,maxBytes:1024})).rejects.toThrow('byte limit');}finally{vi.unstubAllGlobals();}
 });
});
