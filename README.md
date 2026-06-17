[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/mmalmi/nostr-social-graph)

# Nostr Social Graph

> Main development is on [decentralized git](https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-social-graph): `htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-social-graph`

A repository for building and querying Nostr social graphs in both TypeScript and Rust.
The Rust workspace also includes `nostr-identity` for tag-native UUID identity
ops/snapshots and an optional social-memory crate for entity memory, Nostr
attestations, counter-attestations, and trust scoring.

## Features

- Build social graphs from Nostr follow events
- Query followed users, followers, and follow distances
- Change social graph root user with efficient distance recalculation
- Low memory consumption
- Efficient binary serialization (55% smaller than JSON)
- Pre-crawled datasets
- Server for maintaining and serving the up-to-date social graph, for quick initialization in web apps
- Rust workspace with interchangeable in-memory and LMDB-backed backends
- Tag-native UUID identity/entity op and snapshot helpers
- Optional social memory for entities, key/identifier attestations, and trust scoring

## Usage

Choose one path:

- TypeScript app: install `nostr-social-graph`, hydrate from binary or start empty, feed kind `3` and `10000` events, query distances/follows, persist back to binary.
- Rust service/job: use `nostr-social-graph` for in-memory graphs or `nostr-social-graph-heed` for a persistent LMDB-backed graph.

TypeScript:

```ts
import { SocialGraph, type NostrEvent } from "nostr-social-graph";

const root = "<hex pubkey>";
const graph = new SocialGraph(root);

graph.handleEvent(nostrEvent as NostrEvent, true);
console.log(graph.getFollowDistance("<other pubkey>"));

const binary = await graph.toBinary();
const restored = await SocialGraph.fromBinary(root, binary);
```

Rust:

```rust
use nostr_social_graph::SocialGraph;

let mut graph = SocialGraph::new("<hex pubkey>");
graph.handle_event(&event, true, 1.0);
println!("{}", graph.get_follow_distance("<other pubkey>"));
```

Notes:

- Unknown authors are ignored unless you pass `allowUnknownAuthors = true`.
- `setRoot` is async in TypeScript. `await graph.setRoot(pubkey)` before reading distances for the new root.
- If you connect a new root into already-loaded graph data, run `recalculateFollowDistances()` / `recalculate_follow_distances()` after that linking batch.

## Repository Layout

- [`ts/`](./ts/): TypeScript package and examples
- [`rust/`](./rust/): Rust workspace with the `nostr-social-graph` core crate and the `nostr-social-graph-heed` LMDB backend

Package-specific docs:

- [`ts/README.md`](./ts/README.md)
- [`rust/README.md`](./rust/README.md)

## Demo & API

- **Demo**: [graph.iris.to](https://graph.iris.to) ([examples dir](./ts/examples/))
- **Documentation**: [mmalmi.github.io/nostr-social-graph/docs](https://mmalmi.github.io/nostr-social-graph/docs/)
- **API Endpoints**:
  - https://graph-api.iris.to/social-graph?maxBytes=2000000
  - https://graph-api.iris.to/profile-data?maxBytes=2000000&noPictures=true
- Used in production at [iris.to](https://iris.to).

To point the examples search at a hashtree index, set `VITE_PROFILE_SEARCH_INDEX=nhash1qqsgm4ex4d4dxgz39hj6q7t7ax7u4k57gp2zkjuxtfga7wpw6dy6xpg9yqu6y09zecw9hzettkaulu928dt58ndt0h2exw6qg5kxyrprucz0cukym2c` (and optionally `VITE_BLOSSOM_SERVERS=url1,url2`).
Latest published profile search index (2025-01-23): `nhash1qqsgm4ex4d4dxgz39hj6q7t7ax7u4k57gp2zkjuxtfga7wpw6dy6xpg9yqu6y09zecw9hzettkaulu928dt58ndt0h2exw6qg5kxyrprucz0cukym2c`.
To publish the profile search index to Blossom, run `BLOSSOM_NSEC=... pnpm publish-profile-index`.

## Core Implementation

The TypeScript implementation lives in [SocialGraph.ts](./ts/src/SocialGraph.ts), and the Rust workspace lives under [`rust/`](./rust/).

The Rust workspace now has two interchangeable backends:

- [`rust/crates/nostr-social-graph`](./rust/crates/nostr-social-graph): in-memory core graph and binary format
- [`rust/crates/nostr-social-graph-heed`](./rust/crates/nostr-social-graph-heed): optional LMDB/`heed` backend for persistent large graphs
- [`rust/crates/nostr-identity`](./rust/crates/nostr-identity): UUID identity/entity ops, snapshots, projection, and Nostr event helpers
- [`rust/crates/nostr-social-memory`](./rust/crates/nostr-social-memory): entity memory, attestations, counter-attestations, and trust scoring

Both Rust backends implement the shared `SocialGraphBackend` runtime trait from the core crate.
