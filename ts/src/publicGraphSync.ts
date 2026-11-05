import { verifyEvent, type Event } from 'nostr-tools';
import { SocialGraph } from './SocialGraph';
import { Negentropy, NegentropyStorageVector } from './vendor/negentropy.js';

export interface GraphSyncStats {
  sentBytes: number; receivedBytes: number; receivedEvents: number;
  appliedEvents: number; batches: number;
  /** Always zero: public graph catch-up requires NIP-77. Kept for compatibility. */
  fallbackBatches: number;
  elapsedMs: number; limited: boolean; failedRelays: number; completedAuthors: number;
}
export interface GraphSyncOptions {
  graph: SocialGraph;
  relays: string[];
  authors?: string[];
  signal?: AbortSignal;
  maxAuthors?: number;
  maxBytes?: number;
  timeoutMs?: number;
  maxDurationMs?: number;
  onChange?: () => void;
  onEvent?: (event:Event) => void;
  /** Injection for Node and deterministic protocol tests. */
  webSocket?: typeof WebSocket;
}
const hex = /^[0-9a-f]{64}$/;
const bytes = (s:string) => new TextEncoder().encode(s).byteLength;
const bounded = (n:number, min:number, max:number) => {
  if (!Number.isInteger(n) || n<min || n>max) throw new RangeError('Invalid graph sync budget');
  return n;
};
export function graphSyncAuthors(graph:SocialGraph):string[] {
  const {ids} = graph.getInternalData();
  // Public identities only; private contact node IDs never leave this adapter.
  return [...new Set([graph.getRoot(), ...graph.getFollowedByUser(graph.getRoot()), ...Array.from(ids, ([,key])=>key)])].filter(key=>hex.test(key));
}

/** Optional NIP-77 catch-up. Only verified winning public lists alter this graph.
 * Failed reconciliation skips the relay; ordinary queries fetch exact missing IDs only.
 * No snapshots are downloaded and no local records are uploaded or deleted.
 * A fulfilled call can be budget-limited; inspect stats and schedule later work.
 */
export async function syncGraphLists(options:GraphSyncOptions):Promise<GraphSyncStats> {
  const started = performance.now();
  const stats:GraphSyncStats={sentBytes:0,receivedBytes:0,receivedEvents:0,appliedEvents:0,batches:0,fallbackBatches:0,elapsedMs:0,limited:false,failedRelays:0,completedAuthors:0};
  const maxAuthors=bounded(options.maxAuthors??500,1,50000), maxBytes=bounded(options.maxBytes??2*1024*1024,1024,32*1024*1024);
  const timeout=bounded(options.timeoutMs??15000,100,60000);
  const allAuthors=[...new Set(options.authors??graphSyncAuthors(options.graph))].filter(x=>hex.test(x));
  const authors=allAuthors.slice(0,maxAuthors);
  const overallDeadline=Date.now()+bounded(options.maxDurationMs??30000,100,120000);
  const Socket=options.webSocket??globalThis.WebSocket;
  if (!Socket) throw new Error('WebSocket is unavailable');
  let serial=0;
  const abort=()=>options.signal?.throwIfAborted();
  try {
    for(const url of [...new Set(options.relays)].slice(0,3)) {
      abort(); if(stats.limited)break;
      if(Date.now()>=overallDeadline){stats.limited=true;break;}
      const parsed=new URL(url);
      if(parsed.protocol!=='wss:' && !(parsed.protocol==='ws:' && ['localhost','127.0.0.1','[::1]'].includes(parsed.hostname)))throw new Error('Graph sync requires a secure relay');
      const socket=new Socket(url);
      let terminal:Error|undefined, waiting:(()=>void)|undefined;
      const queue:unknown[][]=[];
      const fail=(error:Error)=>{terminal=error;waiting?.();};
      const cancel=()=>{fail(new Error('Graph sync aborted'));socket.close();};
      options.signal?.addEventListener('abort',cancel,{once:true});
      socket.onmessage=message=>{
        if(typeof message.data!=='string')return fail(new Error('Unexpected graph sync frame'));
        stats.receivedBytes+=bytes(message.data);
        if(stats.sentBytes+stats.receivedBytes>maxBytes || message.data.length>512*1024 || queue.length>=512){stats.limited=true;fail(new Error('Graph sync budget reached'));socket.close();return;}
        try{const value=JSON.parse(message.data);if(Array.isArray(value))queue.push(value);waiting?.();}catch{fail(new Error('Invalid relay frame'));}
      };
      socket.onerror=()=>fail(new Error('Graph relay connection failed'));
      socket.onclose=()=>fail(new Error('Graph relay disconnected'));
      const send=(value:unknown[])=>{
        abort();const raw=JSON.stringify(value),size=bytes(raw);
        if(stats.sentBytes+stats.receivedBytes+size>maxBytes){stats.limited=true;throw new Error('Graph sync budget reached');}
        stats.sentBytes+=size;socket.send(raw);
      };
      const next=async(deadline:number):Promise<unknown[]>=>{
        for(;;){abort();if(terminal)throw terminal;if(Date.now()>=overallDeadline)throw new Error('Graph pass timed out');if(queue.length)return queue.shift()!;
          const remaining=Math.min(deadline,overallDeadline)-Date.now();if(remaining<=0)throw new Error('Graph relay timed out');
          await new Promise<void>(resolve=>{const timer=setTimeout(()=>{waiting=undefined;resolve();},remaining);waiting=()=>{clearTimeout(timer);waiting=undefined;resolve();};});
        }
      };
      try {
        await new Promise<void>((resolve,reject)=>{const timer=setTimeout(()=>reject(new Error('Graph relay connect timeout')),Math.min(timeout,Math.max(1,overallDeadline-Date.now())));socket.onopen=()=>{clearTimeout(timer);resolve();};waiting=()=>{clearTimeout(timer);reject(terminal??new Error('Graph relay unavailable'));};});waiting=undefined;
        for(let offset=0;offset<authors.length;offset+=256){
          const batch=authors.slice(offset,offset+256), allowed=new Set(batch), filter={kinds:[3,10000],authors:batch};
          const storage=new NegentropyStorageVector();
          for(const author of batch)for(const kind of [3,10000] as const){
            const revision=options.graph.getListRevision(author,kind);
            if(revision?.complete && hex.test(revision.id))storage.insert(revision.created_at,revision.id);
          }
          storage.seal();const ne=new Negentropy(storage,16384), id=`g${++serial}`;
          const needed=new Set<string>();
          send(['NEG-OPEN',id,filter,await ne.initiate()]);
          const deadline=Date.now()+timeout;
          let rounds=0;
          for(;;){
            const frame=await next(deadline);if(frame[1]!==id)continue;
            if(frame[0]==='NEG-ERR')throw new Error('Graph reconciliation rejected');
            if(frame[0]!=='NEG-MSG' || typeof frame[2]!=='string')continue;
            if(++rounds>64)throw new Error('Graph reconciliation round limit');
            if(!/^(?:[0-9a-f]{2})+$/.test(frame[2]) || frame[2].length>65536)throw new Error('Invalid reconciliation message');
            const [reply,,need]=await ne.reconcile(frame[2]);
            for(const eventId of need){if(hex.test(eventId))needed.add(eventId);}
            if(needed.size>2048){stats.limited=true;throw new Error('Too many graph revisions');}
            if(reply===null)break;
            send(['NEG-MSG',id,reply]);
          }
          send(['NEG-CLOSE',id]);
          stats.batches++;
          const requests=Array.from(needed).reduce<{ids:string[]}[]>((all,eventId,i)=>{if(i%128===0)all.push({ids:[]});all[all.length-1].ids.push(eventId);return all;},[]);
          for(const request of requests){
            const pending=new Set(request.ids);
            const req=`g${++serial}`;send(['REQ',req,request]);const until=Date.now()+timeout;
            for(;;){const frame=await next(until);if(frame[1]!==req)continue;
              if(frame[0]==='CLOSED')throw new Error('Graph event request rejected');
              if(frame[0]==='EOSE'){
                if(pending.size)throw new Error('Graph relay omitted requested revisions');
                break;
              }
              if(frame[0]!=='EVENT')continue;
              stats.receivedEvents++;
              const event=frame[2] as Event;
              if(!event||!allowed.has(event.pubkey)||![3,10000].includes(event.kind)||!Array.isArray(event.tags)||event.tags.length>20000||!needed.has(event.id))continue;
              // Verification can reject malformed fields by throwing.
              let applied=false;
              try{
                if(verifyEvent(event)){
                  // A verified stale revision satisfies the fetch even when a
                  // newer local list wins. Invalid or missing bodies do not.
                  pending.delete(event.id);
                  applied=options.graph.handleEvent(event,true);
                }
              }catch{/* invalid event */}
              if(applied){stats.appliedEvents++;options.onEvent?.(event);}
            }
            send(['CLOSE',req]);
          }
          stats.completedAuthors=Math.max(stats.completedAuthors,offset+batch.length);
        }
      }catch(error){if(options.signal?.aborted)throw error;stats.failedRelays++;/* try another configured relay within the same budget */}
      finally{options.signal?.removeEventListener('abort',cancel);socket.onclose=null;socket.onerror=null;socket.onmessage=null;socket.close();}
    }
  }finally{stats.limited ||= allAuthors.length>maxAuthors || Date.now()>=overallDeadline;stats.elapsedMs=performance.now()-started;if(stats.appliedEvents)options.onChange?.();}
  return stats;
}

export interface GraphSyncLifecycleOptions extends Omit<GraphSyncOptions,'graph'|'signal'|'authors'> {
  getGraph:()=>SocialGraph;
  intervalMs?:number;
  onError?:(error:unknown)=>void;
}
/** Active-tab catch-up with a rotating bounded author window. Caller persists onChange. */
export function startGraphSync(options:GraphSyncLifecycleOptions):()=>void {
  const interval=bounded(options.intervalMs??15*60*1000,1000,24*60*60*1000);
  let stopped=false, controller:AbortController|undefined, timer:ReturnType<typeof setTimeout>|undefined, offset=0;
  const active=()=>typeof document==='undefined'||!document.hidden;
  const run=async()=>{
    if(stopped||controller||!active()||(typeof navigator!=='undefined'&&navigator.onLine===false))return;
    clearTimeout(timer);controller=new AbortController();
    try{
      const graph=options.getGraph(), authors=graphSyncAuthors(graph), count=options.maxAuthors??500;
      if(offset>=authors.length)offset=0;
      const batch=authors.slice(offset,offset+count);
      const result=await syncGraphLists({...options,graph,authors:batch,signal:controller.signal});
      offset=(offset+result.completedAuthors)%Math.max(1,authors.length);
    }catch(error){if(!controller.signal.aborted)options.onError?.(error);}
    finally{controller=undefined;if(!stopped)timer=setTimeout(run,interval);}
  };
  const change=()=>{if(!active())controller?.abort();else void run();};
  globalThis.addEventListener?.('online',change);
  globalThis.document?.addEventListener('visibilitychange',change);
  void run();
  return()=>{stopped=true;clearTimeout(timer);controller?.abort();globalThis.removeEventListener?.('online',change);globalThis.document?.removeEventListener('visibilitychange',change);};
}

/** Fresh discovery bootstrap only. Caller decides whether local state is absent.
 * This unsigned projection is not evidence of an event signature or ownership.
 */
export async function loadGraphBootstrap(options:{root:string;url?:string;signal?:AbortSignal;maxBytes?:number}):Promise<SocialGraph> {
  if(!hex.test(options.root))throw new Error('Invalid public graph root');
  const maxBytes=bounded(options.maxBytes??4*1024*1024,1024,4*1024*1024);
  const timeout=AbortSignal.timeout(8000);
  const signal=options.signal?AbortSignal.any([options.signal,timeout]):timeout;
  const url=new URL(options.url??'https://graph-api.iris.to/social-graph/mobile');
  url.searchParams.set('root',options.root);
  const response=await fetch(url,{credentials:'omit',signal});
  if(!response.ok||!response.body)throw new Error('Graph bootstrap unavailable');
  if(response.headers.get('x-graph-root')!==options.root)throw new Error('Graph bootstrap root mismatch');
  const reader=response.body.getReader(),chunks:Uint8Array[]=[];let length=0;
  try{for(;;){const {value,done}=await reader.read();if(done)break;length+=value.length;if(length>maxBytes)throw new Error('Graph bootstrap byte limit');chunks.push(value);}}
  catch(error){await reader.cancel().catch(()=>{});throw error;}finally{reader.releaseLock();}
  const data=new Uint8Array(length);let offset=0;for(const chunk of chunks){data.set(chunk,offset);offset+=chunk.length;}
  const graph=await SocialGraph.fromBinary(options.root,data);
  const size=graph.size();
  if(!graph.getListRevision(options.root,3)&&!graph.getFollowedByUser(options.root).size)throw new Error('Graph bootstrap has no discovery root list');
  if(Array.from(graph.getInternalData().ids).length>30000||size.users>30000||size.follows+size.mutes>180000)throw new Error('Graph bootstrap node limit');
  return graph;
}
