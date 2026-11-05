import {describe,it,expect} from 'vitest';
import {WebSocket} from 'ws';
import {SocialGraph} from '../src/SocialGraph';
import {syncGraphLists} from '../src/publicGraphSync';
import {graphSyncRelay,signedGraphLists} from './helpers/graphSyncRelay';

const {authors,events}=signedGraphLists();
const socket=WebSocket as unknown as typeof globalThis.WebSocket;
describe('exact-ID graph fetches on capped relays',()=>{
 it('completes signed follow and mute lists when each REQ returns at most 100 events',async()=>{
  const relay=await graphSyncRelay(events,100),graph=new SocialGraph(authors[0]);
  try{
   const stats=await syncGraphLists({graph,authors,relays:[relay.url],webSocket:socket});await relay.settled();
   expect(stats.failedRelays).toBe(0);expect(stats.limited).toBe(false);
   expect(stats.receivedEvents).toBe(events.length);expect(stats.appliedEvents).toBe(events.length);
   expect(stats.completedAuthors).toBe(authors.length);expect(stats.fallbackBatches).toBe(0);
   for(const event of events)expect(graph.getListRevision(event.pubkey,event.kind as 3|10000)).toMatchObject({id:event.id,complete:true});
   const reads=relay.requests.filter(frame=>frame[0]==='REQ') as [string,string,{ids:string[]}][];
   expect(reads).toHaveLength(Math.ceil(events.length/64));
   expect(reads.every(frame=>frame[2].ids.length<=64&&Object.keys(frame[2]).join()==='ids')).toBe(true);
   expect(relay.requests.filter(frame=>frame[0]==='NEG-OPEN')).toHaveLength(1);
   expect(relay.requests.some(frame=>frame[0]==='EVENT')).toBe(false);
   expect(relay.errors).toEqual([]);
   const warm=await syncGraphLists({graph,authors,relays:[relay.url],webSocket:socket});
   expect(warm.receivedEvents).toBe(0);expect(warm.completedAuthors).toBe(authors.length);expect(warm.failedRelays).toBe(0);
  }finally{await relay.close();}
 });
 it('preserves byte-budget failure and does not advance an incomplete author window',async()=>{
  const relay=await graphSyncRelay(events,100),graph=new SocialGraph(authors[0]);
  try{
   const stats=await syncGraphLists({graph,authors,relays:[relay.url],webSocket:socket,maxBytes:30000});
   expect(stats.limited).toBe(true);expect(stats.completedAuthors).toBe(0);expect(stats.failedRelays).toBe(1);
   expect(stats.appliedEvents).toBeLessThan(events.length);expect(stats.fallbackBatches).toBe(0);
  }finally{await relay.close();}
 });
 it('still aborts during event admission without fetching further chunks',async()=>{
  const relay=await graphSyncRelay(events,100),graph=new SocialGraph(authors[0]),controller=new AbortController();let applied=0;
  try{
   await expect(syncGraphLists({graph,authors,relays:[relay.url],webSocket:socket,signal:controller.signal,onEvent:()=>{applied++;controller.abort();}})).rejects.toThrow();
   await relay.settled();expect(applied).toBe(1);expect(relay.requests.filter(frame=>frame[0]==='REQ')).toHaveLength(1);
   expect(relay.requests.some(frame=>frame[0]==='EVENT')).toBe(false);
  }finally{await relay.close();}
 });
});
