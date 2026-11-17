import { SocialGraph, chooseTrustedAuthors, type TrustedAuthorSelection } from 'nostr-social-graph';
import { createRankingSnapshot } from 'nostr-social-graph/ranking';

declare const selection: TrustedAuthorSelection;
const graph = new SocialGraph(selection.rootPubkey);
const authors = chooseTrustedAuthors(selection);
createRankingSnapshot(graph);

// @ts-expect-error The constructor must retain its type when declaration checks are skipped.
const invalidConstructor: symbol = SocialGraph;
// @ts-expect-error Trusted authors are public keys.
authors.add(42);
