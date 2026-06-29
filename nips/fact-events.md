# NIP-XX

## Fact Events

`draft` `optional`

Fact events are tag-native statements about one stable UUID subject. They let
Nostr apps publish small signed facts, update or dispute previous facts, and
materialize trusted views without putting every application data model into a
new event kind.

## What This Is For

Many apps need a way to say things like:

- this UUID represents the same entity as `github:alice`
- this pubkey controls this UUID entity
- this review was written by this reviewer about this place
- this app key is part of this profile's key roster

Nostr already gives us signed events, relay-searchable tags, and local trust
decisions. What is missing is a simple envelope for subject-scoped facts that
different apps can reuse.

This NIP defines that envelope:

- one canonical UUID subject per event
- facts encoded directly as tags, so they are easy to index and filter
- append-only operations for changes over time
- optional addressable snapshots for compact current state
- explicit links for previous operations, replacements, and disputes
- local trust: consumers decide which authors, relays, snapshots, and
  predicates they accept

This NIP does not define global truth, global uniqueness, or a universal
predicate registry. Profiles can define their own predicates on top of the base
event shape.

## Mental Model

A fact operation says:

> The event author asserts these facts about this UUID subject.

The event `pubkey` is the author of the claim. The subject is the entity the
claim is about. Different authors can publish facts about the same subject, and
consumers choose which authors and predicates to trust.

Single-character tags are indexes or references. Multi-character tags are facts.

## Kinds

- `7368`: fact operation
- `37368`: fact snapshot

## Subject

Each fact event MUST have exactly one subject tag:

```json
["i", "<subject>", "subject"]
```

The subject MUST be a canonical lowercase hyphenated UUID string.

Example:

```json
["i", "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a", "subject"]
```

The subject is not the event author. It is the entity, review, roster, or other
object being described.

## Content

Generic fact operations and snapshots MUST have empty `content`.

Profiles that need private payloads MAY define extension events with non-empty
encrypted `content`, but those events MUST still expose enough public tags to
identify the subject and extension `type`. Generic fact parsers MAY ignore such
extension events unless they implement that profile.

The current identity link-request extension uses kind `7368` with NIP-44
encrypted `content`; its public tags identify only the subject, extension
`type`, and invite pubkey.

## Index Tags

These single-character tags are reserved for indexing and references:

- `i`: subject and other searchable identifiers
- `p`: pubkeys
- `e`: event links
- `d`: snapshot address

Single-character tags are not facts. Unknown single-character tags SHOULD be
ignored by fact parsers. `d`, `e`, `i`, and `p` MUST NOT be used as fact
predicates.

`i` tags without the `subject` marker are index-only. They make an event easier
to find, but they do not assert a fact.

Example:

```json
[
  ["i", "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a", "subject"],
  ["i", "github:alice"],
  ["i", "https://github.com/alice"],
  ["same_as", "github:alice"]
]
```

In this example, `same_as` is the fact. The other non-subject `i` tags are
searchable context.

## Fact Tags

Fact tags use a multi-character predicate as the tag name:

```json
["<predicate>", "<value-1>", "<value-2>", "..."]
```

The predicate MUST be at least two characters and MUST NOT contain whitespace.
The remaining tag fields are ordered string values. For simple binary facts,
the first value is the object. Additional values can qualify the fact.

UUID and pubkey values SHOULD be bare canonical strings, without `uuid:` or `p:`
prefixes.

Examples:

```json
["name", "Alice"]
["controls", "<pubkey>"]
["same_as", "github:alice"]
["member_of", "<uuid>"]
["not_member_of", "<uuid>"]
["rating", "4", "5"]
```

## Operation Links

Kind `7368` events MAY link other fact operations:

```json
["e", "<event-id>", "", "prev"]
["e", "<event-id>", "", "replace"]
["e", "<event-id>", "", "dispute"]
```

- `prev`: previous operation by the same author for the same subject
- `replace`: operation this author intends to supersede
- `dispute`: operation this author disputes

Kind `7368` operation events MUST NOT use a `d` tag.

## Snapshots

Kind `37368` events are addressable snapshots of a subject's materialized facts.
They MUST include:

```json
["d", "<subject>"]
["i", "<subject>", "subject"]
```

The `d` tag MUST equal the subject.

Snapshot events MAY link operation heads:

```json
["e", "<event-id>", "", "head"]
```

Snapshot tags SHOULD be deduplicated and sorted before signing.

Snapshot parsers MAY ignore `expiration` as snapshot metadata rather than a fact
predicate.

A snapshot is a convenience, not universal truth. Consumers still decide whether
they trust the snapshot author and the facts inside it.

## Projection

To materialize a subject from operations, consumers SHOULD:

1. Keep only operations for the target subject.
2. Sort operations by `created_at`, then event id.
3. Remove operations referenced by accepted `replace` links.
4. Apply the remaining facts.
5. Record accepted `dispute` links as metadata. A `dispute` link does not remove
   facts by itself.
6. Use `prev` links to compute operation heads.

This leaves conflict resolution to the consumer's trust policy. For example, a
client may accept only facts from selected authors, prefer recent operations, or
show disputed facts with warnings.

## Profiles

The base format intentionally does not reserve most predicates. A profile can
define a vocabulary for a domain by specifying:

- expected `type` facts, if any
- allowed or required predicates
- how predicates map to application data
- any profile-specific validation rules

Consumers SHOULD ignore predicates and profiles they do not understand.

## UUID Entity Profile

This profile describes a durable entity that may be known by external
identifiers and controlled by Nostr pubkeys.

Suggested predicates:

- `name`
- `controls`
- `same_as`
- `member_of`
- `not_member_of`

Example:

```json
{
  "kind": 7368,
  "content": "",
  "tags": [
    ["i", "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a", "subject"],
    ["i", "github:alice"],
    ["p", "4f355bdcb7c27f8f3e4f6c4ec8f996d45b33f2d9c93a9d5c7aa9a6e1a4e7f8aa"],
    ["name", "Alice"],
    ["same_as", "github:alice"],
    ["controls", "4f355bdcb7c27f8f3e4f6c4ec8f996d45b33f2d9c93a9d5c7aa9a6e1a4e7f8aa"]
  ]
}
```

## Review Profile

This profile treats each review as its own UUID subject. The reviewed object and
reviewer can be external identifiers, UUID subjects, or profile-defined values.

Suggested predicates:

- `type`: `review`
- `reviewer`: reviewer identifier or UUID
- `review_of`: reviewed identifier or UUID
- `rating`: value and maximum
- `body`: review text
- `lang`: language code
- `published_at`: unix timestamp
- `source`: source name
- `source_id`: source review id

Crawlers SHOULD add `i` tags for the source review id, source URL, reviewer id,
and reviewed object id.

Example:

```json
{
  "kind": 7368,
  "content": "",
  "tags": [
    ["i", "8ef4ad1f-6d74-4f1a-8f4e-4d7a79a78645", "subject"],
    ["i", "google:review:abc123"],
    ["i", "https://maps.example/review/abc123"],
    ["i", "google:user:alice"],
    ["i", "google:place:xyz"],
    ["type", "review"],
    ["reviewer", "google:user:alice"],
    ["review_of", "google:place:xyz"],
    ["rating", "4", "5"],
    ["body", "Great coffee, noisy on weekends."],
    ["lang", "en"],
    ["published_at", "1716508800"],
    ["source", "google"],
    ["source_id", "google:review:abc123"]
  ]
}
```

## Nostr Identity Profile

The current `nostr-identity` implementation defines profile identity events on
top of fact operations. This section documents that implemented profile; generic
fact-event consumers do not need to implement it.

Roster operations and facet acceptance events use kind `7368` with empty
`content`. Link requests use kind `7368` with encrypted `content`.

### Roster Operation

Roster operations have these public facts:

- `type`: `nostr_identity_roster_op`
- `schema`: `1`
- `actor_pubkey`: signer pubkey
- `actor_seq`: optional integer
- `client_nonce`: caller nonce
- `created_at`: event timestamp as decimal text
- `op`: one of `add_key`, `tombstone_key`, `set_key_capabilities`,
  `rotate_secret_epoch`, `repair_secret_wraps`

Operation-specific facts:

- `add_key`: `key_pubkey`, optional `key_subject`, zero or more `key_purpose`,
  zero or more `key_capability`, `key_added_at`, optional `key_label`
- `tombstone_key`: `target_pubkey`, optional `reason`
- `set_key_capabilities`: `target_pubkey`, zero or more `capability`
- `rotate_secret_epoch` and `repair_secret_wraps`: `secret_epoch`, zero or more
  `wrapped_secret` facts with `[pubkey, wrapped]`

Supported neutral capabilities are `admin`, `write`, `recover`,
`receive_secret_wraps`, and `decrypt_secret_epochs`. Supported neutral purposes
are `app`, `recovery`, `remote_signer`, and `profile`.

### NostrIdentity API Names

The app-facing `NostrIdentity` API maps neutral keys to facets:

- purposes: `app_key`, `recovery_phrase`, `nip46_signer`, `social_profile`
- capabilities: `can_write_roots`, `can_admin_profile`,
  `can_recover_app_keys`, `can_receive_secret_wraps`,
  `can_decrypt_secret_epochs`
- ops: `add_facet`, `tombstone_facet`, `set_capabilities`,
  `rotate_secret_epoch`, `repair_secret_wraps`

App-key labels are not published as `key_label` facts. Device labels are carried
only by the optional `encrypted_device_labels` extension fact, whose payload is
app-encrypted. Non-app facets such as `social_profile` MAY publish `key_label`.

### Facet Acceptance

Facet self-acceptance events are fact operations with:

- `type`: `nostr_identity_key_acceptance`
- `schema`: `1`
- `key_pubkey`: accepting facet pubkey, which MUST equal the event signer
- one or more `purpose`
- optional `roster_op_id`
- `client_nonce`
- `accepted_at`, which MUST equal event `created_at`

### Link Request

Link requests are profile-defined extension events with encrypted content:

- `type`: `nostr_identity_link_request`
- subject `i` tag: profile UUID
- `p` tag: invite pubkey
- `content`: NIP-44 ciphertext encrypted from the joining key to the invite key

The encrypted JSON contains `identity`, `admin_pubkey`, `invite_pubkey`,
`joining_pubkey`, `client_nonce`, `requested_at`, and optional `label`.
`requested_at` MUST equal event `created_at`; `joining_pubkey` MUST equal the
event signer; `invite_pubkey` MUST match the public `p` tag.
