import {SocialGraph} from '../src/SocialGraph';
import {syncGraphLists} from '../src/publicGraphSync';
const root='4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0';
const graph=new SocialGraph(root);
for(const relay of ['wss://graph-relay.iris.to','wss://relay.damus.io']){
 const first=await syncGraphLists({graph,authors:[root],relays:[relay],timeoutMs:5000});
 const second=await syncGraphLists({graph,authors:[root],relays:[relay],timeoutMs:5000});
 console.log(JSON.stringify({relay,first,second,revisions:graph.getListRevisions().length}));
}
