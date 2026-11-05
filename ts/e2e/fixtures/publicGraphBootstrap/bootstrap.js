import { SocialGraph } from 'nostr-social-graph';
import { loadGraphBootstrap, syncGraphLists } from 'nostr-social-graph/publicGraphSync';

const root = '1'.repeat(64);
// Consumers also use the default entry to read a retained graph. Importing both
// entries exposes namespace tree-shaking regressions in the downstream bundle.
let graph = new SocialGraph(root);

export async function bootstrap(url) {
  graph = await loadGraphBootstrap({ root, url });
  const roundtrip = await SocialGraph.fromBinary(root, await graph.toBinary());
  return {
    root: roundtrip.getRoot(),
    follows: [...roundtrip.getFollowedByUser(root)],
    mutes: [...roundtrip.getMutedByUser(root)],
    followRevision: roundtrip.getListRevision(root, 3),
    muteRevision: roundtrip.getListRevision(root, 10000),
  };
}

export async function syncRetained({ root, binary, relays }) {
  const retained = await SocialGraph.fromBinary(root, new Uint8Array(binary));
  const stats = await syncGraphLists({ graph: retained, authors: [root], relays,
    timeoutMs: 500, maxDurationMs: 3000 });
  const saved = await retained.toBinary();
  const roundtrip = await SocialGraph.fromBinary(root, saved);
  return {
    stats, binary: [...saved],
    followRevision: roundtrip.getListRevision(root, 3),
    muteRevision: roundtrip.getListRevision(root, 10000),
  };
}
