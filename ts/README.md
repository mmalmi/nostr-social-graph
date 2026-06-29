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

Key paths:

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
