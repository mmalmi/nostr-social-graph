import {describe, expect, it} from 'vitest';
import {SocialGraph} from '../src/SocialGraph';
const key = (n:number) => n.toString(16).padStart(64,'0');
const event = (kind:number, id=2, at=100, targets=[1,2]) => ({pubkey:key(0),kind,id:key(id),created_at:at,tags:targets.map(n=>['p',key(n)]),content:'',sig:''});
describe('list revisions', () => {
  it.each([2,3,4])('rejects truncated and overlong version %i binary integers promptly', async version => {
    await expect(SocialGraph.fromBinary(key(0),new Uint8Array([version,0,255,255,255,255,7]))).rejects.toThrow();
    await expect(SocialGraph.fromBinary(key(0),new Uint8Array([version,...Array(20).fill(255)]))).rejects.toThrow();
  });
  it.each([3,10000])('keeps winning kind %i revision, including empty lists', async kind => {
    const g = new SocialGraph(key(0));
    expect(g.handleEvent(event(kind))).toBe(true);
    expect(g.handleEvent(event(kind,3))).toBe(false);
    expect(g.handleEvent(event(kind,1,100,[]))).toBe(true);
    expect(g.handleEvent(event(kind,1,100,[]))).toBe(false);
    const restored = await SocialGraph.fromBinary(key(0), await g.toBinary());
    expect(restored.getListRevision(key(0),kind)).toEqual({id:key(1),created_at:100,complete:true});
    expect(restored.handleEvent(event(kind,0,99))).toBe(false);
  });
  it.each([3,10000])('marks bounded kind %i projection incomplete and repairs the same event', async kind => {
    const g = new SocialGraph(key(0));g.handleEvent(event(kind));
    const restored = await SocialGraph.fromBinary(key(0),await g.toBinary(undefined,1));
    expect(restored.getListRevision(key(0),kind)).toEqual({id:key(2),created_at:100,complete:false});
    expect(restored.handleEvent(event(kind))).toBe(true);
    expect(restored.getListRevision(key(0),kind)?.complete).toBe(true);
  });
  it('does not let a newer partial server projection erase a client crawl', async () => {
    const client = new SocialGraph(key(0));client.handleEvent(event(3));
    const server = new SocialGraph(key(0));server.handleEvent(event(3,4,101,[3,4]));
    await client.merge(await SocialGraph.fromBinary(key(0),await server.toBinary(undefined,1)));
    expect([...client.getFollowedByUser(key(0))]).toEqual([key(1),key(2)]);
  });
  it('does not advertise manually changed edges as complete', () => {
    const g = new SocialGraph(key(0));g.handleEvent(event(3));g.addFollower(key(0),key(3));
    expect(g.getListRevision(key(0),3)?.complete).toBe(false);
  });
});
