# Nostr Social Graph TS

This directory contains the published TypeScript package for building and querying Nostr social graphs.

Quickstart:

```ts
import { SocialGraph, type NostrEvent } from "nostr-social-graph";

const graph = new SocialGraph(rootPubkey);
graph.handleEvent(nostrEvent as NostrEvent, true);

const binary = await graph.toBinary();
const restored = await SocialGraph.fromBinary(rootPubkey, binary);
```

Binary snapshots preserve observed empty follow and mute lists with their event
timestamps, so older lists cannot reappear after restoring a snapshot. Budgeted
exports retain these empty facts for included owners within the distance limit;
an omitted or truncated nonempty list is never recorded as an empty list.

Notes:

- `handleEvent` only uses kind `3` and `10000` events.
- Fact-event helpers export tag-native UUID op/snapshot builders, parsers, and projection utilities.
- `NostrIdentity*` exports are the app-facing identity roster protocol: AppKey facets, profile secret epochs, wrapped secrets, parent-id projection, and encrypted payload tag helpers such as `encrypted_device_labels`.
- Lower-level `IdentityGraph*` exports are the neutral fact-roster primitives used by `NostrIdentity`.
- Unknown authors are ignored by default. Pass `true` when ingesting from a cold start.
- `await graph.setRoot(pubkey)` before reading follow distances for a new root.
- If you connect a new root into preloaded graph data, run `recalculateFollowDistances()` before reading distances.

## Nondelegated author authority

Applications can separate admission (for example group membership) from whose
signals affect a trusted tally or ranking:

```ts
import {
  chooseTrustedAuthors,
  countDistinctTrustedAuthors,
} from "nostr-social-graph";

const trusted = chooseTrustedAuthors({
  rootPubkey,
  eligibleAuthors: eligibleMemberKeys,
  directFollows: graph.getFollowedByUser(rootPubkey),
  mutedAuthors: graph.getMutedByUser(rootPubkey),
});
const trustedSignals = countDistinctTrustedAuthors(signalAuthorKeys, trusted);
```

Only the eligible root and its eligible direct follows receive authority.
Following, endorsing, or admitting a descendant does not delegate that authority.
A compromised trusted account therefore contributes at most its own signal even
if it admits thousands of accounts. Inputs are authenticated application state;
event verification, membership consent, roster/policy snapshots, ballot choice,
and closure rules belong to the application. Distinct keys are not proof of
distinct people, and a compromised root can explicitly trust new accounts in a
new view. The helper does not claim resistance to that trust-anchor failure or a
colluding majority of explicitly trusted authors.

The returned set is an independent, sorted snapshot. Persisting it inside a
signed poll can keep later contact or membership edits from changing that poll's
electorate; signatures alone do not establish completeness or election finality.
The production graph tests cover 1000 and 10000 admitted Sybils, repeated signals,
reordered input, trust revocation, and the expected compromised-root boundary.

## Optional ranking

Scoring is a separate import. Loading `SocialGraph`, updating follows, and
reading distances never computes ranks or loads the ranking module.

```ts
import { createRankingSnapshot, calculateRanking } from "nostr-social-graph/ranking";

// Run explicitly in an application worker after selecting the graph root.
const snapshot = createRankingSnapshot(graph);
const ranks = calculateRanking(snapshot, { algorithm: "pagerank", damping: 0.5 });
const score = ranks.get(authorPubkey); // unknown or excluded: 0
console.log(ranks.converged, ranks.iterations, ranks.workingArrayBytes);
```

PageRank splits each profile's influence among its followed profiles. A walk
restarts at the selected root; dangling mass also returns there. Root-muted
profiles cannot receive or pass influence. Another author's mute overrides
their follow edge. These are recommendation scores, not identities, voting
rights, group authority, or replacements for application visibility filters.
`directSeed` describes the root's direct contribution to one follow, so an app
can choose a reference scale without assigning each additional key a full vote.

`algorithm: "graperank"` provides an experimental capped weighted average using
positive follows and zero-valued mutes, with `attenuation` and `rigor` controls.
It uses the equations credited in [ranking-NOTICE.md](src/ranking-NOTICE.md),
with simultaneous updates and global convergence. It is not a bit-for-bit port
of the upstream service. A cap per profile does **not** conserve a cluster's
combined influence; the adversarial tests demonstrate that limitation.

Preparation defaults to at most 250,000 reachable profiles and 2,000,000
inspected edges, rejecting oversized inputs rather than silently truncating.
Calculation defaults to at most 64 iterations and 64 million node/edge visits.
Reaching the calculation limit returns `converged: false`; applications decide
whether to use that approximation. Parameters are validated. Preparation is
O(V + E), calculation O(iterations × (V + E)), and memory O(V + E).

The module has no networking, timers, storage, graph mutation, or runtime
dependencies. Cache results by graph root and application graph revision;
recompute on follow/mute changes, not on each displayed event. Retain neither
old roots nor old revisions indefinitely. `workingArrayBytes` counts numeric
buffers only; string lookup Maps and the caller's graph use additional memory.
Snapshot fields are read-only by contract. Scores reflect the supplied snapshot,
which may omit followers or complete follow lists; do not silently mix scores
from different graph coverage. Existing distance and mute APIs are unchanged.

Benchmark a local snapshot with
`node --expose-gc --import tsx scripts/benchmarkRanking.ts SNAPSHOT ROOT`.
An optional third argument names a previous ranking module to compare. The
benchmark first checks exact snapshot and score parity, then alternates baseline
and candidate measurements after warmup. Use a quiet machine and compare both
preparation and calculation; improvements depend on the graph and JavaScript
runtime. The calculation's work budget remains a conservative bound even when
zero-mass sources can skip their edges.

## Package paths and commands

- [`src/`](./src/): library source
- [`tests/`](./tests/): package tests
- [`examples/`](./examples/): demo app
- [`server/`](./server/): graph/profile API server

Common commands:

- `pnpm test`
- `pnpm build`
- `pnpm docs`
- `pnpm e2e`

Package metadata lives in [`package.json`](./package.json). For repo-wide context and the Rust workspace, see the [root README](../README.md).

## Published bootstrap snapshot

`data/socialGraph.bin` is a checksum-pinned public bootstrap rooted at Sirius
Business Ltd. Its adjacent `socialGraph.json` records the root, capture source,
input/output hashes and coverage. It retains complete follow and mute lists for
owners through two hops, including their third-hop targets and empty-list
timestamps. Applications should preserve explicitly selected personal roots.
The library constructor still takes an explicit root; this file is seed data,
not a current or complete account graph.

`projectSocialGraph(source, root, ownerDistance)` canonicalizes numeric aliases
by public identity before traversing and assigning fresh IDs. The newest list
timestamp wins. Conflicting equally recent follow lists reject; equally recent
mute lists conservatively union. This policy avoids silently losing mutes when
a server aggregate contains duplicate mappings. Binary snapshots are trusted
aggregate state and do not contain individually verifiable event signatures.

Run `pnpm snapshot:verify` to check the committed artifact. Explicitly reproduce
it with `pnpm snapshot:refresh /path/to/pinned-source.bin` (or omit the source
path to fetch the manifest's versioned URL). Input and output checksums must
match before writing. To adopt a newer source, review and update the manifest
and coverage together. Ordinary packaging only verifies the reviewed bytes;
it never fetches a mutable snapshot or replaces them with a local crawl.

Versioned releases provide `socialGraph.bin` and its manifest separately, so
native and web consumers can pin the same data without upgrading their SDK.

## Optional public graph updates

See [public graph sync](./PUBLIC_GRAPH_SYNC.md) for optional NIP-77 follow/mute
catch-up, bounded mobile bootstraps, local snapshot preservation and measured
wire overhead. The default library import does not open network connections.
