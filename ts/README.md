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
