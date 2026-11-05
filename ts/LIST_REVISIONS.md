# Follow and mute list revisions

`SocialGraph` records the latest accepted event ID and timestamp separately for
kind 3 and kind 10000. Ingestion still expects signature-verified events; neither
an event ID nor a downloaded binary snapshot proves a signature was verified.
Unsigned local edge edits and bounded projections are not complete event copies.

```ts
const revision = graph.getListRevision(pubkey, 3);
const items = graph.getListRevisions()
  .filter(r => r.complete)
  .map(r => ({id: r.id, timestamp: r.created_at}));
```

Use `complete` revisions to construct optional NIP-77 reconciliation sets. An
unknown ID (including all legacy snapshots) must not be invented. A partial list
must not enter the set either: otherwise the relay sees an event as already held
and the client never restores the omitted follows/mutes. After receiving and
verifying the complete event, `handleEvent` can restore a partial copy even when
the ID and timestamp match. Lower timestamps cannot roll back newer local state;
for equal timestamps the lexicographically lower event ID wins.

`getListRevisions()` returns `{pubkey, kind, id, created_at, complete}` records.
`getListRevision(pubkey, kind)` returns the revision without pubkey/kind, or
`undefined`. `markListIncomplete(pubkey, kind)` is required after callers alter a
list through low-level maps or project an event before ingestion. Normal
`addFollower`/`removeFollower` mark the affected follow list incomplete themselves.
Public mute tags are included; private encrypted mute contents are not decrypted.

Snapshots use binary v4, retaining v2/v3 read support. Each serialized list gets
one byte (`0` unknown revision, `1` partial, `2` complete); known revisions add a
32-byte event ID. The existing list timestamp supplies `created_at`. The marker
and optional ID follow the timestamp and precede the edge count. Node and edge
budgets continue to bound exports, and truncating an already partial list never
makes it complete. Empty lists retain their revisions too. At 30,000 known lists,
IDs add 960,000 bytes before compression, plus one marker byte per stored list.

Clients should load their saved graph first and use server snapshots only as
bootstrap or explicitly chosen discovery data. `merge` does not replace a local
list with an incoming partial list. Keep personal local overlays separate from
verified state. Persist accepted revisions and edges together, and use ordinary
live subscriptions alongside reconciliation; Negentropy is catch-up, not push.

Rust exposes `get_list_revision`, `get_list_revisions`, and
`mark_list_incomplete` on the in-memory graph. Heed and Hashtree preserve revisions
across durable checkpoints. `SocialGraphBackend::get_list_revisions` defaults to
an empty set for older custom backends. Rust servers can retain legacy clients
with `to_binary_with_budget_version(budget, 3)` while publishing v4 at a separate
endpoint. Old readers cannot decode v4; upgrade the library before switching
snapshot endpoints.
