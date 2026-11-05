import {describe,expect,it} from 'vitest';
import {SocialGraph} from '../src/SocialGraph';
import type {NostrEvent} from '../src/utils';

const key=(n:number)=>n.toString(16).padStart(64,'0');
const event=(author:number,kind:number,created_at:number,targets:number[]=[]):NostrEvent=>({
  pubkey:key(author),kind,created_at,tags:targets.map(n=>['p',key(n)]),content:'',id:'fixture',sig:'fixture',
});

describe('known-empty list persistence',()=>{
  it.each([3,10000])('preserves a first empty kind%d list and rejects stale state after restore',async(kind)=>{
    const graph=new SocialGraph(key(0));
    expect(graph.handleEvent(event(0,kind,2000))).toBe(true);
    const before=graph.getInternalData();
    // Timestamp-only state is a legitimate result of first-empty ingestion.
    expect((kind===3?before.followListCreatedAt:before.muteListCreatedAt).get(before.ids.id(key(0)))).toBe(2000);
    const bytes=await graph.toBinary();
    const restored=await SocialGraph.fromBinary(key(0),bytes);
    expect(kind===3?restored.getFollowListCreatedAt(key(0)):restored.getMuteListCreatedAt(key(0))).toBe(2000);
    expect(restored.handleEvent(event(0,kind,1999,[1]))).toBe(false);
    expect(restored.handleEvent(event(0,kind,2000,[1]))).toBe(false);
    expect(kind===3?restored.getFollowedByUser(key(0)).size:restored.getMutedByUser(key(0)).size).toBe(0);
    expect(restored.handleEvent(event(0,kind,2001,[1]))).toBe(true);
    expect(kind===3?restored.getFollowedByUser(key(0)).has(key(1)):restored.getMutedByUser(key(0)).has(key(1))).toBe(true);
  });

  it('retains empty facts for disconnected users in a complete snapshot without inventing facts for unknown users',async()=>{
    const graph=new SocialGraph(key(0));
    graph.addFollower(key(0),key(1));
    graph.handleEvent(event(2,3,2000),true);
    graph.handleEvent(event(2,10000,2001),true);
    const restored=await SocialGraph.fromBinary(key(0),await graph.toBinary());
    expect(restored.getFollowListCreatedAt(key(2))).toBe(2000);
    expect(restored.getMuteListCreatedAt(key(2))).toBe(2001);
    expect(restored.getFollowListCreatedAt(key(1))).toBeUndefined();
    expect(restored.getMuteListCreatedAt(key(1))).toBeUndefined();
    expect(restored.getFollowDistance(key(2))).toBe(1000);
  });

  it.each([[2,1],[undefined,undefined,1],[0,0,undefined,0]])('retains known-empty facts for selected owners under budgets %j',async(...limits)=>{
    const graph=new SocialGraph(key(0));
    graph.handleEvent(event(0,3,1000,[1]));
    graph.handleEvent(event(1,3,2000));
    graph.handleEvent(event(0,10000,3000));
    const restored=await SocialGraph.fromBinary(key(0),await graph.toBinary(...limits));
    expect(restored.getFollowListCreatedAt(key(1))).toBe(2000);
    expect(restored.getMuteListCreatedAt(key(0))).toBe(3000);
    expect(restored.handleEvent(event(1,3,1999,[2]))).toBe(false);
  });

  it('preserves an isolated root empty list within a one-node budget',async()=>{
    const graph=new SocialGraph(key(0));graph.handleEvent(event(0,3,2000));
    const restored=await SocialGraph.fromBinary(key(0),await graph.toBinary(1));
    expect(restored.getFollowListCreatedAt(key(0))).toBe(2000);
  });

  it('does not export out-of-distance facts or mislabel a nonempty list excluded by node limits',async()=>{
    const graph=new SocialGraph(key(0));
    graph.handleEvent(event(0,3,1000,[1]));graph.handleEvent(event(1,3,2000));graph.handleEvent(event(0,10000,3000));
    const near=await SocialGraph.fromBinary(key(0),await graph.toBinary(undefined,undefined,0));
    expect(near.getFollowListCreatedAt(key(0))).toBe(1000);
    expect(near.getFollowListCreatedAt(key(1))).toBeUndefined();
    expect(near.getMuteListCreatedAt(key(0))).toBe(3000);
    const single=await SocialGraph.fromBinary(key(0),await graph.toBinary(1));
    expect(single.getFollowListCreatedAt(key(0))).toBeUndefined();
    expect(single.getFollowListCreatedAt(key(1))).toBeUndefined();
    expect(single.getMuteListCreatedAt(key(0))).toBe(3000);
  });
});
