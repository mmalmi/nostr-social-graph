# Optional public graph catch-up

Import `nostr-social-graph/publicGraphSync` only in applications that own a public
social graph. The default package entry adds no sockets, polling, timers or
Negentropy dependency. Native applications can use their existing NIP-77 SDK
with `get_list_revisions()` instead of implementing another protocol.

```ts
import {startGraphSync} from 'nostr-social-graph/publicGraphSync';
const stop = startGraphSync({
  getGraph: () => myRetainedGraph,
  relays: myPublicRelays,
  onChange: () => persistAndRefresh(myRetainedGraph),
});
// stop() on logout, graph disposal or application teardown.
```

The helper compares both kind 3 follow lists and kind 10000 public mute tags,
fetches missing full events, verifies signatures, and applies winning revisions.
It never publishes local records, decrypts mute content, or replaces the graph.
It uses direct configured public relays; do not route these author queries over
private-account or peer-scoped transports. `syncGraphLists` provides a one-shot
variant, AbortSignal, byte/time budgets and transfer statistics. Partial failures
are reported as `failedRelays`, budget exhaustion as `limited`; successful return
does not mean every relay or every author was fully synchronized.

Defaults: 500 authors per pass in 256-author filters, 2 MiB total application
traffic, 15-second batch deadlines, a 30-second total pass deadline, at most three configured relays, and a
15-minute rotating catch-up interval while visible and online. A single arriving
WebSocket frame can cross the transfer budget before the socket is closed.
Relays without NIP-77 fall back to bounded ordinary list requests. Keep existing
live list subscriptions for immediate updates; reconciliation repairs missed or
late-arriving events. A fresh server bootstrap reduces missing events, but does
not remove the author-filter overhead.

## Snapshot and local-data rules

Prefer a valid locally crawled graph on startup. Download a server snapshot only
when local state is absent, or on explicit user reset/import. New snapshots use
binary v4 and retain each known list's accepted ID, timestamp and completeness.
Only complete lists belong in the Negentropy set. Projected/truncated lists keep
provenance IDs but must be downloaded again before claiming complete sync. Legacy
v2/v3 snapshots have unknown revision IDs and need an initial reconciliation.
Never treat a relay missing an event as a deletion. Equal-time revisions use the
lexicographically smaller ID. Keep graph and revision metadata in one durable
snapshot; preserve empty lists and private node identity boundaries.

For mobile bootstraps the server provides `/social-graph/mobile`: at most 30,000
nodes, 180,000 edges, 500 edges per owner and 4 MiB uncompressed. This is a discovery
projection, not a substitute for verified event signatures. Its byte cap is a
hard response cap, not an assurance that every graph can be represented within
that cap. `/healthz` measures crawler progress independently of whether users
have recently changed their lists.

## Overhead measurement

Run `pnpm --filter nostr-social-graph exec tsx scripts/benchmarkGraphSync.ts`.
The deterministic reference-protocol test includes author filters, JSON framing,
hex encoding and missing-event REQ IDs; excludes TLS/WebSocket overhead and
actual event payloads. At 30,000 authors (60,000 follow/mute heads):

| Authors with both lists changed | Missing events | Reconciliation + request bytes |
| ---: | ---: | ---: |
| 0 | 0 | 2,093,308 |
| 30 | 60 | 2,103,909 |
| 300 | 600 | 2,221,495 |
| 3,000 | 6,000 | 3,393,809 |

This is why clients should not reconcile the whole network on every page load.
A 500-author unchanged pass is roughly 35 KB per relay, before transport overhead.
With a fresh persisted graph, only changed lists require event downloads. An
initial import lacking revision IDs is more expensive; keep the same budgets
and resume in subsequent passes. The benchmark's 0.24–0.35s local wall times are
not mobile CPU or real-network latency measurements. The script also reports
illustrative event-payload cost separately so it cannot be confused with measured
network traffic. `scripts/probeGraphSync.ts` exercises real configured public
relays twice to check cold and unchanged catch-up behavior.

A live one-author probe against the dedicated graph relay downloaded two list
events (30,451 received bytes) on first sync and zero events (157 received bytes,
275 sent bytes) on repeat. That relay was behind a general relay at probe time;
protocol efficiency is not evidence of freshness. Deployment acceptance must
include crawler and relay-forward progress, not only a successful handshake.
