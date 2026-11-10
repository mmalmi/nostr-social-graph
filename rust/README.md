# Nostr Social Graph Rust

This workspace contains the Rust implementation of the Nostr social graph,
tag-native fact events with opaque subjects, and an optional social-memory crate for entity
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
- Follow distances stay exact as lists change: a follow or unfollow updates everyone reached through that edge, including when a new root links into preloaded graph data. `set_root` recalculates them itself; `recalculate_follow_distances()` is only needed for imported state whose distances were never computed.
- Binary snapshots retain observed empty follow/mute lists and their timestamps, preventing stale lists from returning after reload. Budgeted snapshots keep these facts for retained, in-range owners; truncated nonempty lists are not labeled empty.

Crates:

- [`crates/nostr-social-graph`](./crates/nostr-social-graph): in-memory core graph, binary format, and shared `SocialGraphBackend` trait
- [`crates/nostr-social-graph-heed`](./crates/nostr-social-graph-heed): optional LMDB/`heed` backend implementing the same runtime trait
- [`crates/nostr-identity`](./crates/nostr-identity): Fact ops with opaque subjects, neutral `IdentityGraph` roster primitives, and app-facing `NostrIdentity` AppKey/secret-epoch event builders, parsers, projection, parent-id, and encrypted payload helpers
- [`crates/nostr-social-memory`](./crates/nostr-social-memory): UUID-backed entities, Nostr attestations/counter-attestations, and trust scoring over a graph

## Dependency minimums

Version 0.2.1 of the Hashtree backend and social-memory package requires
`nostr-social-graph` 0.2.1 or a later compatible 0.2.x release, carrying the
alias-preserving snapshot import fix. Consumers still using either package at
0.2.0 must update that dependency to receive the corrected minimum.

The server also requires at least 0.2.1 of the core, Heed and Hashtree backends.
These minimums belong in package manifests: a corrected workspace lock alone
does not exclude core 0.2.0 from another consumer's existing lockfile.

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
directory as `socialGraph.large.bin` and `profileData.large.json`. The next startup performs a full graph reconciliation; file modification time
is not treated as evidence that events have been fetched. An old binary export beside the LMDB database
can be months behind the running graph and must not be used for this migration.

On first startup, the new server imports the binary graph into Hashtree. Verify
the imported follows, mutes, timestamps, and profiles against the captured data,
then restart it and verify persistence before switching traffic. Take a final
fresh export for the switch and keep the original LMDB data intact.


### Live freshness and mobile bootstrap

The server subscribes to live public kind-3 and kind-10000 events. It also
reconciles known authors at startup and at least daily without a `since` bound,
so delayed relay events and legacy snapshots without revision IDs can be repaired.
Each author batch is capped at 128 authors and requires EOSE from at least one
upstream relay; disconnected relays cannot turn an empty response into a successful
crawl. Coverage is relative to responding relays, not a claim of Nostr-wide completeness.
Between full passes it uses successful crawl watermarks with a five-minute overlap;
a failed pass never advances that watermark. Graph persistence completes before
profile crawling, and `DATA_DIR/graph-freshness.json` records successful durable
passes. The configured sync interval is a pause **after** a pass, not its duration.

`GET /healthz` is uncached JSON. HTTP 200 requires a running live notification
loop, no outstanding crawl failure, a durable graph pass within two hours, and a
full reconciliation within 48 hours. Startup returns 503 until the first pass
finishes. Monitor this endpoint externally; event timestamps alone cannot measure
freshness because an unchanged follow list can legitimately be old.

`GET /social-graph/mobile` is a discovery bootstrap capped at 30,000 identities,
180,000 edges, four hops, and 500 edges per author, with a hard four-MiB decoded
response limit. It includes the binary format's list revision IDs and completeness
flags. It is cached for 60 seconds with a root-bound SHA-256 ETag and supports
`If-None-Match`. The optional `root` query accepts the configured server root or
the Iris company discovery root; other roots are rejected to bound work/cache
size. `X-Graph-Root` identifies the projection root (exposed through CORS).
The existing `/social-graph` endpoint remains available for custom projections.
Clients must retain their own graph and merge only appropriate newer records;
loading this discovery projection is not authority to replace a user's crawl.
A partial list's ID must never be advertised as a fully synchronized event.


To populate a NIP-77 relay without a duplicate mirror database, optionally set
`GRAPH_FORWARD_RELAY_URL` to a trusted writable relay and
`GRAPH_FORWARD_ALLOWLIST_PATH` to its allowlist file. The crawler refreshes the
allowlist and forwards signature-verified winning public follow/mute events from
both repair passes and live delivery. Its 256-event queue is bounded; live queue
overflow and relay rejections appear in `/healthz`. The target relay must enforce
Nostr replaceable-event retention (one winning event per author/kind).
`GRAPH_FORWARD_MIN_FREE_BYTES` defaults to five GiB and guards the data filesystem
(which must also contain the target relay database); forwarding stops before
consuming that reserve. Protected events and events beyond the relay's 512-KiB /
25,000-tag admission limits are intentionally not forwarded. Clients should retain
other relay sources for these records. First complete forwarding reconciliation is
reported separately from graph crawl freshness.


Completed crawl distances create explicitly **partial** durable checkpoints;
these never advance the full-pass watermark or make `/healthz` healthy. The
checkpoint writer first budgets filesystem growth from the serialized snapshot
and refuses to consume `GRAPH_CHECKPOINT_MIN_FREE_BYTES` (five GiB by default).
`nostr-social-graph-server --check-checkpoint` reloads and verifies the durable
Hashtree checkpoint and prints counts/revision coverage without connecting to
relays. This allows deployment acceptance to verify recovery while a larger full
repair is still in progress.
