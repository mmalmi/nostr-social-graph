# Private contact memory

[Back to the overview](../README.md)

`ContactMemory` in Rust's `nostr-social-memory` crate and the TypeScript
`contactMemory` helpers share the JSON format in
[`fixtures/contact-memory.json`](../fixtures/contact-memory.json). Apps keep one
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

## Queued sibling preference sync (v2)

Use `nostr-social-graph/privateContactSyncV2` and
`nostr-social-graph/privateContactSyncV2Controller`; Rust exports matching
`*_v2` functions and `*V2` types from `nostr-social-memory`. The portable fixture
is [`fixtures/private-contact-sync-v2.json`](../fixtures/private-contact-sync-v2.json).

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

## Unsigned private events

`nostr-social-graph/privateContactFacts` and Rust `nostr_identity` provide matching
kind-7368 events for private nicknames, notes, favorite and muted fields. These
are ordinary Nostr events with `id`, `pubkey`, `created_at`, `kind`, `tags` and
`content`, without `sig`. IDs use the standard Nostr event hash. Each contact
field retains its original counter/writer stamp; null and false are explicit
clears. A fixed `created_at: 0` makes migration and repair IDs deterministic;
field stamps determine merge order. Only an authenticated active sibling
transport may admit these events. The pubkey alone is not authentication.

Version-4 durable state stores facts and pending edits atomically. Restoration
preserves V1/V2 register state and migrates the former V3 draft wrappers without
changing clocks or losing queued clears. Reconcile facts after negotiating
`privateFacts:2`; encrypted V2 contact controls remain readable by older peers.

`nostr-social-graph/privateEvents` also builds and validates ordinary unsigned
events and account-setting facts. Setting facts use the shared fact builders,
with separate setting and target tags and a canonically ordered JSON value.
Keep independent targets independent during merge: deleting one chat must not
replace another chat's deletion. Per-chat deletion cutoffs merge by maximum;
newer messages remain eligible. Callers supply each setting's validation and
conflict rules. Legacy signed events are verified before their signatures are
removed; public authorization proofs must retain their signatures.

Never send private events, IDs or tags to public relays or sign them for public
publication. Public name attestations require a separate, explicit action;
personal nicknames do not approve or verify someone's name.

## Legacy encrypted preference sync (v1, compatibility only)

This retired format lacks forward secrecy: compromise of the long-lived owner key
can decrypt retained self-encrypted records. It is not recommended for new writes.

`nostr-social-graph/privateContactSync` and
`nostr-social-graph/privateContactSyncController` provide the shared TypeScript
core; Rust's `nostr-social-memory` exports matching `private_contact_*` helpers.
The interoperable fixture is [`fixtures/private-contact-sync.json`](../fixtures/private-contact-sync.json).
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
