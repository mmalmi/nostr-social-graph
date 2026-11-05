import {once} from 'node:events';
import {WebSocket,WebSocketServer} from 'ws';
import {finalizeEvent,getPublicKey,type Event} from 'nostr-tools';
import {Negentropy,NegentropyStorageVector} from '../../src/vendor/negentropy.js';

export function signedGraphLists(authorCount=130) {
 const authors:string[]=[],events:Event[]=[];
 for(let i=1;i<=authorCount;i++){
  const secret=new Uint8Array(32);new DataView(secret.buffer).setUint32(28,i);
  const pubkey=getPublicKey(secret);authors.push(pubkey);
  for(const kind of [3,10000])events.push(finalizeEvent({kind,created_at:1700000000+i,content:'',tags:[['p',pubkey]]},secret));
 }
 return {authors,events};
}

/** Loopback relay with real NIP-77; caps event bodies per exact-ID request. */
export async function graphSyncRelay(events:Event[],eventCap=Infinity){
 const server=new WebSocketServer({port:0,host:'127.0.0.1'});await once(server,'listening');
 const requests:unknown[][]=[],sockets=new Set<WebSocket>(),errors:string[]=[];
 const wire={sentBytes:0,receivedBytes:0,authorFilterBytes:0,eventPayloadBytes:0,exactIdRequestBytes:0};
 server.on('connection',socket=>{
  sockets.add(socket);const sessions=new Map<string,Negentropy>();let pending=Promise.resolve();
  const send=(frame:unknown[])=>{const raw=JSON.stringify(frame);wire.receivedBytes+=Buffer.byteLength(raw);socket.send(raw);};
  socket.on('message',raw=>{
   wire.sentBytes+=Buffer.byteLength(raw.toString());
   pending=pending.then(async()=>{
    const frame=JSON.parse(raw.toString());requests.push(frame);
    if(frame[0]==='NEG-OPEN'){
     wire.authorFilterBytes+=Buffer.byteLength(JSON.stringify(frame[2]));
     const storage=new NegentropyStorageVector();
     for(const event of events)if(frame[2].authors.includes(event.pubkey)&&frame[2].kinds.includes(event.kind))storage.insert(event.created_at,event.id);
     storage.seal();const session=new Negentropy(storage,16384);sessions.set(frame[1],session);
     send(['NEG-MSG',frame[1],(await session.reconcile(frame[3]))[0]]);
    }else if(frame[0]==='NEG-MSG')send(['NEG-MSG',frame[1],(await sessions.get(frame[1])!.reconcile(frame[2]))[0]]);
    else if(frame[0]==='NEG-CLOSE')sessions.delete(frame[1]);
    else if(frame[0]==='REQ'){
     wire.exactIdRequestBytes+=Buffer.byteLength(JSON.stringify(frame));
     const filter=frame[2];
     if(!Array.isArray(filter.ids)||Object.keys(filter).some(key=>key!=='ids'))throw new Error('Fixture forbids broad graph requests');
     for(const event of events.filter(event=>filter.ids.includes(event.id)).slice(0,eventCap)){
      wire.eventPayloadBytes+=Buffer.byteLength(JSON.stringify(event));send(['EVENT',frame[1],event]);
     }
     send(['EOSE',frame[1]]);
    }else if(frame[0]==='EVENT')throw new Error('Fixture forbids uploads');
   }).catch(error=>{errors.push(String(error));socket.close();});
  });
 });
 return {url:`ws://127.0.0.1:${(server.address() as {port:number}).port}`,requests,wire,errors,
  async settled(){await Promise.all([...sockets].filter(socket=>socket.readyState!==WebSocket.CLOSED).map(socket=>once(socket,'close')));},
  async close(){for(const socket of sockets)socket.terminate();await new Promise<void>(resolve=>server.close(()=>resolve()));}
 };
}
