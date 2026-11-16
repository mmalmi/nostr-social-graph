import { expect, it } from 'vitest';
import { SocialGraph } from '../src/SocialGraph';
import { openGraphSnapshot } from '../src/hashtreeSnapshot';
import { readMobileGraphSnapshot, verifyMobileGraphSnapshot } from '../src/mobileGraphSnapshot';
import { namedGraphFixture } from './helpers/namedGraph';

it('pins the exact signed mobile member and verifies partial and empty revisions offline', async () => {
  const root = '1'.repeat(64);
  const neighbor = '2'.repeat(64);
  const graph = new SocialGraph(root);
  graph.handleEvent({pubkey:root, kind:3, tags:[['p',neighbor]], id:'a'.repeat(64),created_at:10,content:'',sig:''});
  graph.handleEvent({pubkey:neighbor, kind:10000, tags:[], id:'b'.repeat(64),created_at:11,content:'',sig:''});
  graph.markListIncomplete(root, 3);
  const bytes = await graph.toBinary();
  const fixture = await namedGraphFixture(root, bytes);
  let closed = false;
  try {
    const snapshot = await openGraphSnapshot(fixture.options);
    expect(() => { snapshot.manifest.mobile_roots![root].size = 0; }).toThrow();
    await expect(readMobileGraphSnapshot('f'.repeat(64), {snapshot})).rejects.toThrow('no mobile projection');
    await expect(readMobileGraphSnapshot(root, {snapshot, treeName:'other'})).rejects.toThrow('name mismatch');
    const mobile = await readMobileGraphSnapshot(root, {snapshot});
    expect(fixture.filters).toHaveLength(1);
    expect(fixture.requests).toHaveLength(3);
    expect(mobile.bytes).toEqual(bytes);
    expect(mobile.graph.getListRevision(neighbor,10000)).toMatchObject({id:'b'.repeat(64),complete:true});
    expect(mobile.receipt).toMatchObject({root,version:2,bytes:bytes.length,directFollows:1,secondDegree:0,
      revisionCounts:{follows:{ids:1,complete:0},mutes:{ids:1,complete:1}},
      source:{publisher:fixture.options.publisher, treeName:'social-graph', eventId:snapshot.eventId,
        directoryHash:snapshot.directoryHash, file:`mobile-${root}.bin`, bytes:bytes.length}});
    await fixture.close();
    closed = true;
    const frozen = await verifyMobileGraphSnapshot(bytes, mobile.receipt);
    expect(frozen.getListRevision(root,3)).toMatchObject({id:'a'.repeat(64),complete:false});
    const {receipt} = mobile;
    await expect(verifyMobileGraphSnapshot(bytes, {...receipt, users:0})).rejects.toThrow('row count');
    await expect(verifyMobileGraphSnapshot(bytes, {...receipt, revisionCounts:{...receipt.revisionCounts,mutes:{ids:1,complete:0}}})).rejects.toThrow('revision count');
    await expect(verifyMobileGraphSnapshot(bytes, {...receipt,source:{...receipt.source,file:'../mobile.bin'}})).rejects.toThrow('source receipt');
    await expect(verifyMobileGraphSnapshot(bytes.slice(0,-1), receipt)).rejects.toThrow('checksum');
  } finally { if (!closed) await fixture.close(); }
});
