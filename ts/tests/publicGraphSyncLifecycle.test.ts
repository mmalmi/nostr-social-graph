import {afterEach,describe,expect,it,vi} from 'vitest';
import {WebSocket} from 'ws';
import {SocialGraph} from '../src/SocialGraph';
import {startGraphSync,syncGraphLists} from '../src/publicGraphSync';
const key=(n:number)=>n.toString(16).padStart(64,'0');
afterEach(()=>vi.useRealTimers());
describe('graph sync lifecycle bounds',()=>{
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
