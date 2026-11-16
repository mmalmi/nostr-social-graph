import { HashTree, LinkType, MemoryStore, fromHex, sha256, toHex } from '@hashtree/core';
import { finalizeEvent, generateSecretKey, getPublicKey, nip19 } from 'nostr-tools';
import { SocialGraph } from '../SocialGraph';
import { toBinary } from '../SocialGraphBinary';

/** Synthetic signed publication and encrypted block store for a consumer's isolated tests. */
export async function createNamedGraphFixture(root: string, mobile: Uint8Array) {
  const secret = generateSecretKey();
  const store = new MemoryStore();
  const tree = new HashTree({store, encryptionMode: 'legacy-chk'});
  const encoder = new TextEncoder();
  const profile = encoder.encode('[["' + root + '","Alice"]]');
  const primary = getPublicKey(secret);
  const full = await SocialGraph.fromBinary(root, mobile);
  full.handleEvent(finalizeEvent({kind: 3, created_at: 1700000001, tags: [['p', root]], content: ''}, secret), true);
  await full.setRoot(primary);
  const directoryRootMobile = await full.toBinary(30000, 180000, 4, 500);
  const reachable: string[] = [];
  for (let distance = 0; ; distance++) {
    const users = full.getUsersByFollowDistance(distance);
    if (!users.size) break;
    reachable.push(...users);
  }
  const index = { keys: [{path:['name'],id:'name',weight:1,src:'name'}], records:[{i:0,$:{0:{v:'Alice',n:1}}}] };
  const payloads: Record<string, Uint8Array> = {
    'social-graph.bin': await toBinary(full, undefined, undefined, undefined, undefined, 3),
    'social-graph-v4.bin': await full.toBinary(), 'mobile.bin': directoryRootMobile,
    'profile-data.json': profile, 'profile-index.json': encoder.encode(JSON.stringify(index)),
    'allowlist.txt': encoder.encode(reachable.join('\n') + '\n'),
  };
  const projection = 'mobile-' + root + '.bin';
  const files = Object.fromEntries(await Promise.all(Object.entries(payloads).map(async ([name, bytes]) => [
    name, {size: bytes.length, sha256: toHex(await sha256(bytes))},
  ])));
  const manifest = {
    version: 1, root: primary, graph_completed_at: 1700000010, graph_since: 1700000000,
    generated_at: 1700000011, files, mobile_roots: {[root]: {file: projection, size: mobile.length, sha256: toHex(await sha256(mobile))}},
  };
  const uploaded = await Promise.all(Object.entries({...payloads, [projection]: mobile}).map(async ([name, bytes]) => ({
    ...await tree.putFile(bytes), name, type: LinkType.File,
  })));
  const timestamp = Math.floor(Date.now() / 1000) - 60;
  async function signedTree() {
    const entry = await tree.putFile(encoder.encode(JSON.stringify(manifest)));
    const {cid} = await tree.putDirectory([...uploaded, {...entry, name: 'manifest.json', type: LinkType.File}]);
    return finalizeEvent({
      kind: 30064, created_at: timestamp, tags: [['d','social-graph'],['l','hashtree'],['hash',toHex(cid.hash)],['key',toHex(cid.key!)]], content: '',
    }, secret);
  }
  const event = await signedTree();
  return {
    publisher: nip19.npubEncode(primary), event, readBlock: (hashHex: string) => store.get(fromHex(hashHex)),
    manifest, uploaded: uploaded.map(entry => ({name: entry.name, hash: toHex(entry.cid.hash)})), profile, signedTree,
  };
}
