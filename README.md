[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/mmalmi/nostr-social-graph)

# Nostr Social Graph

> Main development is on [decentralized git](https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-social-graph): `htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-social-graph`

A repository for building and querying Nostr social graphs in both TypeScript and Rust.
The Rust workspace also includes `nostr-identity` for tag-native UUID fact
ops/snapshots used by identity, contacts, and rosters, plus an optional
social-memory crate for entity memory, Nostr attestations, counter-attestations,
and trust scoring.

## Features

- Build social graphs from Nostr follow events
- Query followed users, followers, and follow distances
- Change social graph root user with efficient distance recalculation
- Low memory consumption
- Efficient binary serialization (55% smaller than JSON)
- Pre-crawled datasets
- Server for maintaining and serving the up-to-date social graph, for quick initialization in web apps
- Rust workspace with interchangeable in-memory and LMDB-backed backends
- Tag-native UUID fact op and snapshot helpers
- App-facing `NostrIdentity` roster/AppKey helpers with profile secret epochs, wrapped secrets, parent projection, and encrypted device-label payload tags
- Optional social memory for entities, key/identifier attestations, and trust scoring
- Portable private contact names, explicitly approved name changes, and favorites

## Private contact memory

`ContactMemory` in Rust's `nostr-social-memory` crate and the TypeScript
`contactMemory` helpers share the JSON format in
[`fixtures/contact-memory.json`](./fixtures/contact-memory.json). Apps keep one
record per viewing account and contact, using their own SQLite, browser storage,
or private sync. The helpers do not publish events or synchronize data by
themselves. Keep public profile metadata and public follows separate.

After the first interaction, call `observe_name` (Rust) or `observeContactName`
(TypeScript) with the profile name. Missing or blank names can be filled when
metadata arrives. Later observations preserve both the first name and the name
accepted by the viewer. Show `accepted_name` where continuity matters; use
`pending_name` / `pendingContactName` to offer a change. Only explicit
`approve_name` / `approveContactName` calls update the accepted name and append
history, and only when the name the viewer approved still matches the latest
profile. Pass Unix seconds for approval timestamps. A private `favorite` flag
does not add a public follow or confer a social-graph checkmark.

### Queued sibling preference sync (v2)

Use `nostr-social-graph/privateContactSyncV2` and
`nostr-social-graph/privateContactSyncV2Controller`; Rust exports matching
`*_v2` functions and `*V2` types from `nostr-social-memory`. The portable fixture
is [`fixtures/private-contact-sync-v2.json`](./fixtures/private-contact-sync-v2.json).

V2 keeps the existing per-field `{counter,writer,value}` merge rule, including
explicit `false`/`null` clears, and adds private contact `muted`. This is separate
from chat notification mutes and public mute lists. Notes, nicknames, favorites
and mutes remain local-first; remembered public names/history stay separate.

The document is `{version:2,owner,contact,fields}`. Carry it **inside** an
already authenticated encrypted sibling message with inner kind `10452` and
`{type:'private-contact-sync',v:2,document}`. A paired device requests a snapshot
with `{type:'private-contact-sync-request',v:2,owner}` under the same inner kind.
The receiver calls `queuePrivateContactSnapshot` to send all known registers,
including clears, in bounded per-contact documents. These are field-wise merges,
not replacement address-book snapshots. New/restarted peers request recovery;
only original field stamps are forwarded. Never place contacts or values in
public routing tags.

The caller must validate the same owner and an authorized active sibling before
parsing or merging a control. This library does not authenticate a channel.
`mergeTrusted` resolves only after atomic local persistence and never stages an
echo. An inbound message must remain in the messaging runtime's durable journal
until that promise resolves; storage failure or a stopped controller rejects it.

`send(document)` returns true only after an authenticated sibling runtime has
**durably queued** the exact document and owns retries for intended offline
recipients. False or thrown errors retain pending work. Only that exact revision
is dequeued; a concurrent newer edit survives. `ready` means the controller has
handed off its pending work, not that every device received it. Existing ratchet
transport can carry the ciphertext over relays or available FIPS channels; this
library introduces neither another encryption key nor a transport fallback.

`migratePrivateContactSync` / `migrate_private_contact_sync_v2` imports a local V1
state without changing any register stamps or clock, merges pending records, and
queues its full merged snapshot once. It retires sealed events, relay ACKs and
read caches. Stop the old publisher and subscription before migration, then persist V2
atomically before enabling the new controller; never run both publishers. Import uncaptured nondefault local settings only as
counter-zero seeds, so old copies cannot resurrect clears. Existing V1 exports
remain for compatibility, but V2 never signs, decrypts, reads or publishes kind
30078. Do not send V2 data under legacy kind10451, legacy `privateContacts` or
legacy `contactDetails` snapshot fields: old apps can republish those. Use a
versioned snapshot field (for example `privateContactsV2`) with the same strict
V2 parser. Old apps safely ignore the new control instead of receiving private
updates through the retired bridge.

### Legacy encrypted preference sync (v1, compatibility only)

This retired format lacks forward secrecy: compromise of the long-lived owner key
can decrypt retained self-encrypted records. It is not recommended for new writes.

`nostr-social-graph/privateContactSync` and
`nostr-social-graph/privateContactSyncController` provide the shared TypeScript
core; Rust's `nostr-social-memory` exports matching `private_contact_*` helpers.
The interoperable fixture is [`fixtures/private-contact-sync.json`](./fixtures/private-contact-sync.json).
It contains a deliberately public test key, never an account key.

The v1 protocol synchronizes **private favorites, explicitly assigned nicknames,
and notes**. It does not alter the separate first-observed/accepted public names
or their explicit approval history. A nickname is a private override, not an
approval of a public profile change. Public follows and social trust are separate.

- Each independently writable replica has a persistent random 128-bit lowercase
  hex `writer`. Each contact it writes has a persistent random `record_id`.
  Never derive these IDs from the contact, and never share one writable replica
  between concurrent processes without a transaction/lock around the entire state.
- A document is `{version:1,owner,contact,writer,record_id,fields}`. Fields are
  `favorite`, `nickname`, and `note`; each optional register is
  `{counter,writer,value}`. Counters are nonnegative safe integers, incremented
  beyond every observed counter for an explicit local edit. Larger counter,
  then lexicographically larger writer wins independently for each field.
  A final UTF-8 JSON-value comparison makes even duplicated-stamp conflicts
  converge. `false` and `null` are retained tombstones, never missing fields.
- One [NIP-78](https://github.com/nostr-protocol/nips/blob/master/78.md) kind
  `30078` event holds each writer/contact record. Its only tags are
  `['d','nostr-social-memory/v1:<writer>:<record_id>']` and
  `['t','nostr-social-memory/v1']`. Content is the document JSON encrypted to
  the author's own key using [NIP-44](https://github.com/nostr-protocol/nips/blob/master/44.md)
  v2. Contacts and values never appear in public tags. Account, timing, record
  count and approximate size remain visible; encryption is not metadata hiding.
- Different writers use different addresses, so a stale offline device cannot
  replace another device's only stored operations. Each changed contact sends
  one coalesced record, not the whole address book. Keep tombstones and all
  retained writer heads; removing old device heads without a compaction protocol
  can lose data on fresh devices.
- `seedPrivateContact` / `seed_private_contact` imports nondefault legacy data at
  counter zero only when the field is absent. It cannot overwrite a real edit or
  resurrect a remote deletion. Save the migration with the durable replica.
  Legacy timestamp-only sibling snapshots remain seed data after upgrading;
  importing them must not invent a new edit on each echo.
- Nicknames are at most 320 UTF-8 bytes; notes 16,384 bytes. Document JSON is
  bounded at 24,576 bytes and encrypted content at 40,000 bytes. Smaller existing
  UI limits are allowed for new edits, but adapters must preserve valid imported
  values rather than truncate them. Empty text is valid; `null` explicitly clears.

Adapters persist the complete returned state atomically before changing the UI.
Local edits and merges never invoke encryption or network access. The controller
serializes state writes, coalesces sends, and persists the exact signed event
before publishing. Only an explicit remote acknowledgement of that exact event
clears pending work; offline/error/restart retries keep identical ciphertext.
Edits arriving during an acknowledgement remain pending. A new head waits until
the preceding head's second has passed, avoiding NIP-01's same-timestamp ID tie
break without manufacturing future timestamps. Stop the controller and close its
subscriptions before switching accounts. Do not show “synced” until initial
history recovery is complete; use the controller's read-readiness barrier.

Discover events with `privateContactSyncFilter(owner)`: exact author, kind, and
`#t` namespace. There is no `#d` prefix wildcard. Fresh-device recovery must fetch
**all retained namespace heads**, not only events since a saved last-seen time.
Use the transport's completed historical reads with inclusive `until` pagination,
deduplicate IDs, and keep the boundary second in the next page. If a full page
contains only the current boundary second, standard NIP-01 has no event-ID cursor:
report incomplete recovery instead of silently skipping records. Respect known
server caps; an unreported smaller cap cannot establish completeness. Relay
retention/availability and NIP-42 authentication, where required, remain transport
responsibilities. Preserve local state and pending work on read failures.

Validate the owner, exact namespace and signature **before decryption**. Signer
methods are checked against the expected account before and after asynchronous
operations. Extension signers need NIP-44 support; request access deliberately,
and retain the bounded accepted-event cache to avoid repeated decrypt prompts.
Never fall back to unencrypted transport or another account when access fails.

Linked devices without the owner's key can exchange the same documents through
their existing authenticated, encrypted sibling channel. `mergeTrusted` /
`stage_private_contact_document` is **not authentication**: the adapter must first
validate owner and registered device. It preserves original field stamps while
staging a merged contact at the recipient's own random publication address.
`privateContactDocuments` / `private_contact_documents` exports all known fields
for that private snapshot; its contextual record IDs must not be published
directly. A full-key sibling can publish for linked devices. No new identity or
device-authorization protocol is defined here.

These helpers own data and transition rules. Profile fetching/search, avatar
components, checkmark appearance, interaction boundaries, and storage belong to
the application. Cross-app reuse of the format requires an app-owned private
export/import or sync adapter; using the helper alone does not share records.

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
- [`nips/fact-events.md`](./nips/fact-events.md): draft NIP for tag-native fact events and UUID entity profiles

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
- [`rust/crates/nostr-identity`](./rust/crates/nostr-identity): UUID fact ops, `IdentityGraph` roster primitives, and app-facing `NostrIdentity` AppKey/secret-epoch event helpers
- [`rust/crates/nostr-social-memory`](./rust/crates/nostr-social-memory): entity memory, attestations, counter-attestations, and trust scoring

Both Rust backends implement the shared `SocialGraphBackend` runtime trait from the core crate.
