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

## Server upgrades

The server stores its graph in `DATA_DIR/socialGraph.hashtree`. Configure
`RELAY_URLS` explicitly as a comma-separated list of working Nostr relay URLs;
the server has no default relays. The Docker build uses the locked, published
Hashtree dependencies and requires no sibling checkout.

When upgrading a server that used `socialGraph.heed`, retain its data directory
and image for rollback. Export a fresh, unrestricted `/social-graph` snapshot
and `/profile-data` response from the running service into a separate data
directory as `socialGraph.large.bin` and `profileData.large.json`. Preserve the
capture start time as their modification time so the next incremental crawl
covers changes made during capture. An old binary export beside the LMDB database
can be months behind the running graph and must not be used for this migration.

On first startup, the new server imports the binary graph into Hashtree. Verify
the imported follows, mutes, timestamps, and profiles against the captured data,
then restart it and verify persistence before switching traffic. Take a final
fresh export for the switch and keep the original LMDB data intact.
