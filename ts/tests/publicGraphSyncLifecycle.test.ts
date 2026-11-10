import {afterEach,describe,expect,it,vi} from 'vitest';
import {WebSocket} from 'ws';
import {setTimeout as delay} from 'node:timers/promises';
import {graphSyncRelay} from './helpers/graphSyncRelay';
import {SocialGraph} from '../src/SocialGraph';
import {graphSyncAuthors,startGraphSync,syncGraphLists} from '../src/publicGraphSync';
const key=(n:number)=>n.toString(16).padStart(64,'0');
afterEach(()=>vi.useRealTimers());
describe('graph sync lifecycle bounds',()=>{
  it('continues outward after successful passes and restarts with a changed root',async()=>{
    const server=await graphSyncRelay([]);
    vi.useFakeTimers({toFake:['setTimeout','clearTimeout','Date']});
    const graph=new SocialGraph(key(1));
    for(let i=1;i<4;i++)graph.addFollower(key(i),key(i+1));
    const completed=()=>server.requests.filter(frame=>frame[0]==='NEG-CLOSE').length;
    const waitForPass=async(count:number)=>{
      for(let attempt=0;attempt<200&&completed()<count;attempt++)await delay(5);
      expect(completed()).toBe(count);
    };
    const authors=()=>server.requests.filter(frame=>frame[0]==='NEG-OPEN')
      .map(frame=>(frame[2] as {authors:string[]}).authors);
    const stop=startGraphSync({getGraph:()=>graph,maxAuthors:1,relays:[server.url],webSocket:WebSocket as any});
    try {
      await waitForPass(1);
      expect(authors()).toEqual([[key(1)]]);
      await vi.advanceTimersByTimeAsync(60000);await waitForPass(2);
      expect(authors()).toEqual([[key(1)],[key(2)]]);
      await graph.setRoot(key(3));
      await vi.advanceTimersByTimeAsync(60000);await waitForPass(3);
      expect(authors()).toEqual([[key(1)],[key(2)],[key(3)]]);
      await vi.advanceTimersByTimeAsync(60000);await waitForPass(4);
      expect(authors()).toEqual([[key(1)],[key(2)],[key(3)],[key(4)]]);
      expect(server.errors).toEqual([]);
      expect(server.requests.some(frame=>frame[0]==='REQ'||frame[0]==='EVENT')).toBe(false);
    } finally {stop();vi.useRealTimers();await server.close();}
  });

  it('revisits the root and its follows while the rest of the sweep continues outward',async()=>{
    const server=await graphSyncRelay([]);
    vi.useFakeTimers({toFake:['setTimeout','clearTimeout','Date']});
    const graph=new SocialGraph(key(1));
    for(const followed of [2,3,4])graph.addFollower(key(1),key(followed));
    for(let followed=5;followed<=10;followed++)graph.addFollower(key(2),key(followed));
    const passes=()=>server.requests.filter(frame=>frame[0]==='NEG-OPEN')
      .map(frame=>(frame[2] as {authors:string[]}).authors.map(author=>parseInt(author,16)));
    const waitForPass=async(count:number)=>{
      for(let attempt=0;attempt<200&&passes().length<count;attempt++)await delay(5);
      expect(passes()).toHaveLength(count);
    };
    const stop=startGraphSync({getGraph:()=>graph,maxAuthors:4,relays:[server.url],webSocket:WebSocket as any});
    try {
      await waitForPass(1);
      for(let pass=2;pass<=4;pass++){await vi.advanceTimersByTimeAsync(60000);await waitForPass(pass);}
      // Half of each pass cycles through the four closest authors, half moves on.
      expect(passes()).toEqual([[1,2,5,6],[3,4,7,8],[1,2,9,10],[3,4,5,6]]);
      expect(server.errors).toEqual([]);
    } finally {stop();vi.useRealTimers();await server.close();}
  });

  it('orders public authors by distance from the current root, regardless of insertion order',async()=>{
    const graph=new SocialGraph(key(1));
    graph.addFollower(key(9),key(3));
    graph.addFollower(key(3),key(4));
    graph.addFollower(key(1),key(7));
    graph.addFollower(key(1),key(2));
    graph.addFollower(key(2),key(3));
    graph.addFollower(key(2),'private-contact');
    await graph.recalculateFollowDistances(100,1000,()=>{});
    expect(graphSyncAuthors(graph)).toEqual([1,2,7,3,4,9].map(key));
    await graph.setRoot(key(2));
    expect(graphSyncAuthors(graph)).toEqual([2,3,4,1,7,9].map(key));
  });

  it('starts immediately and repeats every minute by default',async()=>{
    vi.useFakeTimers();
    const getGraph=vi.fn().mockReturnValue(new SocialGraph(key(1)));
    const stop=startGraphSync({getGraph,relays:[],webSocket:WebSocket as any});
    try {
      expect(getGraph).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(59999);
      expect(getGraph).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(getGraph).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(60000);
      expect(getGraph).toHaveBeenCalledTimes(3);
    } finally {stop();}
    await vi.advanceTimersByTimeAsync(60000);
    expect(getGraph).toHaveBeenCalledTimes(3);
  });

  it('recovers from an unavailable graph and stops its retry timer',async()=>{
    vi.useFakeTimers();
    const graph=new SocialGraph(key(1));
    const getGraph=vi.fn().mockImplementationOnce(()=>{throw new Error('storage unavailable');}).mockReturnValue(graph);
    const onError=vi.fn();
    const stop=startGraphSync({getGraph,onError,relays:[],webSocket:WebSocket as any,intervalMs:1000});
    try {
      expect(onError).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(1000);
      expect(getGraph).toHaveBeenCalledTimes(2);
    } finally {stop();}
    await vi.advanceTimersByTimeAsync(5000);
    expect(getGraph).toHaveBeenCalledTimes(2);
  });
  it('reports an author budget even when no relay work is available',async()=>{
    const result=await syncGraphLists({graph:new SocialGraph(key(1)),authors:[key(1),key(2)],maxAuthors:1,relays:[],webSocket:WebSocket as any});
    expect(result.limited).toBe(true);expect(result.completedAuthors).toBe(0);
  });
});
