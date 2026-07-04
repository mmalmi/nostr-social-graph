# NIP-XX

## Fact Events

`draft` `optional`

Fact events are tag-native subject-predicate-object statements. They let Nostr
apps publish signed facts about one subject identifier, update or dispute facts,
and materialize trusted views without creating a new event kind for every app
data model.

## What This Is For

Apps often need to say:

- this subject is the same entity as `github:alice`
- this pubkey controls this entity
- this app key was revoked and replaced
- this review is about this place
- this key belongs to this profile roster

This is the same shape used by knowledge graphs and RDF, but expressed as Nostr
events. The event subject is the subject, each multi-character tag name is a
predicate, and the tag values are the object or object arguments. This NIP does
not require RDF syntax, RDF stores, global ontologies, or a reasoning engine.

Persistent non-key subjects matter because identities should not always be tied
to one Nostr key. A person, app profile, organization, business, map location,
review, or imported Web account can keep the same subject while keys, names,
labels, metadata, or current facts change. This also represents entities that
are not on Nostr.

For Nostr identities, this supports per-app or per-device keys instead of one
shared long-lived private key. Users can revoke a compromised key, rotate to a
new key, and keep the same identity subject.

This NIP defines only the reusable envelope: one subject identifier, tag-native
facts, append-only operations, optional snapshots, operation links, and local
trust decisions. It does not define global truth, uniqueness, or a universal
predicate registry.

## Kinds

- `7368`: fact operation
- `37368`: fact snapshot

## Subject And Content

Each fact event MUST have exactly one subject tag:

```json
["i", "<subject>", "subject"]
```

The subject MUST be a non-empty identifier string. For a given profile or
publisher, the same logical entity SHOULD keep the same subject when keys,
names, labels, metadata, or current facts change. Profiles MAY require a
specific subject format, such as canonical lowercase hyphenated UUIDs.

The event `pubkey` is the claim author, not the subject.

Generic fact operations and snapshots MUST have empty `content`. Profiles MAY
define extension events with non-empty encrypted `content`, but they MUST still
expose enough public tags to identify the subject and extension `type`. Generic
fact parsers MAY ignore such extensions.

## Tags

Single-character tags are reserved for indexing and references:

- `i`: subject and other searchable identifiers
- `p`: pubkeys
- `e`: event links
- `d`: snapshot address

`ms` is reserved for snapshot millisecond metadata.

Reserved tags are not facts. Unknown single-character tags SHOULD be ignored by
fact parsers. `d`, `e`, `i`, `ms`, and `p` MUST NOT be used as fact predicates.

`i` tags without the `subject` marker are index-only. They make the event easier
to find, but do not assert a fact.

Fact tags use a multi-character predicate as the tag name:

```json
["<predicate>", "<value-1>", "<value-2>", "..."]
```

Predicates MUST be at least two characters and MUST NOT contain whitespace. Tag
values are ordered strings. For simple binary facts, the first value is the
object; additional values qualify the fact.

UUID and pubkey values SHOULD be bare canonical strings, without `uuid:` or `p:`
prefixes, when a profile uses those value types.

Example:

```json
{
  "kind": 7368,
  "content": "",
  "tags": [
    ["i", "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a", "subject"],
    ["i", "github:alice"],
    ["p", "4f355bdcb7c27f8f3e4f6c4ec8f996d45b33f2d9c93a9d5c7aa9a6e1a4e7f8aa"],
    ["same_as", "github:alice"],
    ["controls", "4f355bdcb7c27f8f3e4f6c4ec8f996d45b33f2d9c93a9d5c7aa9a6e1a4e7f8aa"],
    ["rating", "4", "5"]
  ]
}
```

In this example, `same_as`, `controls`, and `rating` are facts. The non-subject
`i` and `p` tags are searchable indexes.

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

The `d` tag MUST equal the subject. Snapshot events MAY link operation heads:

```json
["e", "<event-id>", "", "head"]
```

Snapshot tags SHOULD be deduplicated and sorted before signing. Snapshot parsers
MAY ignore `expiration` as snapshot metadata rather than a fact predicate.

Snapshots MAY include millisecond metadata:

```json
["ms", "<unix-ms>"]
```

If present, `ms` MUST be a decimal Unix timestamp in milliseconds and
`floor(ms / 1000)` MUST equal event `created_at`. When multiple snapshots for
the same address are available, consumers SHOULD prefer the snapshot with the
larger `created_at`, then larger `ms`, then event id. The `ms` tag helps
clients order snapshots created in the same second; it does not change relay
replacement rules by itself.

Snapshots are conveniences, not universal truth. Consumers still decide whether
to trust the snapshot author and facts.

## Projection

To materialize a subject from operations, consumers SHOULD:

1. Keep only operations for the target subject.
2. Sort operations by `created_at`, then event id.
3. Remove operations referenced by accepted `replace` links.
4. Apply the remaining facts.
5. Record accepted `dispute` links as metadata; disputes do not remove facts.
6. Use `prev` links to compute operation heads.

Conflict resolution is local policy. Clients may accept only selected authors,
prefer recent operations, or show disputed facts with warnings.

## Profiles

The base format intentionally reserves few predicates. A profile can define
application vocabulary, required `type` facts, validation rules, and mappings to
app data. Consumers SHOULD ignore predicates and profiles they do not
understand.

### UUID Entity

Uses a canonical lowercase hyphenated UUID subject for a durable entity that may
outlive keys or represent a non-Nostr entity.

Suggested predicates: `name`, `controls`, `same_as`, `member_of`,
`not_member_of`.

### Review

Treats each review as its own subject. Crawlers MAY use UUID subjects or persistent
source-scoped identifiers. The reviewer and reviewed object can be external
identifiers, UUID subjects, or profile-defined values.

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

Crawlers SHOULD add `i` tags for source review id, source URL, reviewer id, and
reviewed object id.

### Nostr Identity

The current `nostr-identity` implementation defines profile identity events on
top of fact operations. Generic fact-event consumers do not need to implement
this profile.

Roster operations and facet acceptance events use kind `7368` with empty
`content`. Link requests use kind `7368` with encrypted `content`.

Roster operations have:

- `type`: `nostr_identity_roster_op`
- `schema`: `1`
- `actor_pubkey`: signer pubkey
- `actor_seq`: optional integer
- `client_nonce`: caller nonce
- `created_at`: event timestamp as decimal text
- `op`: `add_key`, `tombstone_key`, `set_key_capabilities`,
  `rotate_secret_epoch`, or `repair_secret_wraps`

Operation-specific facts:

- `add_key`: `key_pubkey`, optional `key_subject`, zero or more `key_purpose`,
  zero or more `key_capability`, `key_added_at`, optional `key_label`
- `tombstone_key`: `target_pubkey`, optional `reason`
- `set_key_capabilities`: `target_pubkey`, zero or more `capability`
- `rotate_secret_epoch` / `repair_secret_wraps`: `secret_epoch`, zero or more
  `wrapped_secret` facts with `[pubkey, wrapped]`

Neutral capabilities are `admin`, `write`, `recover`, `receive_secret_wraps`,
and `decrypt_secret_epochs`. Neutral purposes are `app`, `recovery`,
`remote_signer`, and `profile`.

The app-facing `NostrIdentity` API maps:

- purposes to `app_key`, `recovery_phrase`, `nip46_signer`, `social_profile`
- capabilities to `can_write_roots`, `can_admin_profile`,
  `can_recover_app_keys`, `can_receive_secret_wraps`,
  `can_decrypt_secret_epochs`
- ops to `add_facet`, `tombstone_facet`, `set_capabilities`,
  `rotate_secret_epoch`, `repair_secret_wraps`

App-key labels are not published as `key_label` facts. Device labels are carried
only by the optional `encrypted_device_labels` extension fact, whose payload is
app-encrypted. Non-app facets such as `social_profile` MAY publish `key_label`.

Facet self-acceptance events have:

- `type`: `nostr_identity_key_acceptance`
- `schema`: `1`
- `key_pubkey`: accepting facet pubkey, which MUST equal the event signer
- one or more `purpose`
- optional `roster_op_id`
- `client_nonce`
- `accepted_at`, which MUST equal event `created_at`

Link requests are encrypted extension events with:

- `type`: `nostr_identity_link_request`
- subject `i` tag: profile UUID
- `p` tag: invite pubkey
- `content`: NIP-44 ciphertext encrypted from the joining key to the invite key

The encrypted JSON contains `identity`, `admin_pubkey`, `invite_pubkey`,
`joining_pubkey`, `client_nonce`, `requested_at`, and optional `label`.
`requested_at` MUST equal event `created_at`; `joining_pubkey` MUST equal the
event signer; `invite_pubkey` MUST match the public `p` tag.
