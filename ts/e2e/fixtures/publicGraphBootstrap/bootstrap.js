import { SocialGraph } from 'nostr-social-graph';
import { loadGraphBootstrap } from 'nostr-social-graph/publicGraphSync';

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
