# Nostr Social Graph Rust

This workspace contains the Rust implementation of the Nostr social graph,
tag-native UUID fact events, and an optional social-memory crate for entity
continuity.

Quickstart:

```rust
use nostr_social_graph::SocialGraph;

let mut graph = SocialGraph::new("<hex pubkey>");
graph.handle_event(&event, true, 1.0);
let distance = graph.get_follow_distance("<other pubkey>");
```

Use `nostr-social-graph-heed` when you want the same runtime API with LMDB-backed persistence:

```rust
use nostr_social_graph_heed::HeedSocialGraph;

let mut graph = HeedSocialGraph::open("./graph-db", "<hex pubkey>")?;
graph.handle_event(&event, true, 1.0)?;
```

Notes:

- `allow_unknown_authors` defaults to your call site, not the library. Pass `true` during initial ingest if you are not filtering to already-reachable authors.
- If you switch roots or connect a new root into preloaded graph data, recompute distances before reading them.

Crates:

- [`crates/nostr-social-graph`](./crates/nostr-social-graph): in-memory core graph, binary format, and shared `SocialGraphBackend` trait
- [`crates/nostr-social-graph-heed`](./crates/nostr-social-graph-heed): optional LMDB/`heed` backend implementing the same runtime trait
- [`crates/nostr-identity`](./crates/nostr-identity): UUID fact ops, neutral `IdentityGraph` roster primitives, and app-facing `NostrIdentity` AppKey/secret-epoch event builders, parsers, projection, parent-id, and encrypted payload helpers
- [`crates/nostr-social-memory`](./crates/nostr-social-memory): UUID-backed entities, Nostr attestations/counter-attestations, and trust scoring over a graph

Common commands:

- `cargo test --manifest-path rust/Cargo.toml`
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets -- -D warnings`
- `cargo fmt --manifest-path rust/Cargo.toml --all`

For repo-wide context and the TypeScript package, see the [root README](../README.md).
