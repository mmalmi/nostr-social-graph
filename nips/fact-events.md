# NIP-XX

## Fact Events

`draft` `optional`

Subject-predicate-object events for signed facts about one UUID subject.
Generic facts are tag-only. Extensions MAY define non-empty encrypted content
when their public tags still identify the subject and event type.

## Kinds

- `7368`: fact operation
- `37368`: fact snapshot

## Subject

Each event MUST have exactly one subject tag:

```json
["i", "<subject>", "subject"]
```

The event `pubkey` is the claim author. The subject is what the facts are about.

UUID subjects MUST be canonical lowercase hyphenated UUID strings.

## Content

Generic fact operation and snapshot `content` MUST be empty.

An extension that carries private data MAY use non-empty encrypted content. Such
events MUST still include public fact/index tags sufficient to identify the
subject and extension `type`. The current identity link-request extension uses
NIP-44 encrypted `content` and public tags only for `subject`, `type`, and the
invite pubkey.

## Index Tags

Reserved/index tags:

- `i`: subject and other searchable identifiers
- `p`: pubkeys
- `e`: event links
- `d`: snapshot address

Single-character tags are index/reference tags, not facts. Unknown
single-character tags SHOULD be ignored by fact parsers. `d`, `e`, `i`, and `p`
MUST NOT be used as fact predicates.

`i` tags without the `subject` marker are index-only. They do not assert facts.

Example crawled unsigned source:

```json
[
  ["i", "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a", "subject"],
  ["i", "github:alice"],
  ["i", "https://github.com/alice"],
  ["same_as", "github:alice"]
]
```

The `same_as` tag is the fact. The URL `i` tag is only searchable context.

## Fact Tags

Fact tags use a multi-character predicate as the tag name:

```json
["<predicate>", "<object>", "<argument-1>", "..."]
```

Predicates MUST be at least two characters and MUST NOT contain whitespace.

UUID and pubkey values SHOULD be bare canonical strings, without `uuid:` or `p:`
prefixes.

Examples:

```json
["name", "Alice"]
["controls", "<pubkey>"]
["same_as", "github:alice"]
["member_of", "<uuid>"]
["not_member_of", "<uuid>"]
```

## Operation Links

Kind `7368` events MAY link other operations:

```json
["e", "<event-id>", "", "prev"]
["e", "<event-id>", "", "replace"]
["e", "<event-id>", "", "dispute"]
```

- `prev`: previous op by the same author for the same subject
- `replace`: op this author supersedes
- `dispute`: op this author disputes

Kind `7368` operation events MUST NOT use a `d` tag.

## Snapshots

Kind `37368` events MUST include:

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

## Projection

Consumers that materialize a subject SHOULD sort operations by `created_at`, then
event id. A `replace` link removes the referenced operation from the projected
fact set. A `dispute` link records dispute metadata but does not remove facts by
itself. `prev` links identify previous operations by the same author for the same
subject and are used to compute operation heads.

## UUID Entity Profile

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

A review is a subject.

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

## Trust

Consumers decide which authors, relays, snapshots, and predicates they trust.

## Nostr Identity Profile

The current `nostr-identity` implementation defines app-facing profile identity
events on top of fact operations. All event kinds are `7368`.

### Roster Operation

Public facts:

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
- `rotate_secret_epoch` / `repair_secret_wraps`: `secret_epoch`, zero or more
  `wrapped_secret` facts with `[pubkey, wrapped]`

Supported neutral capabilities are `admin`, `write`, `recover`,
`receive_secret_wraps`, and `decrypt_secret_epochs`. Supported neutral purposes
are `app`, `recovery`, `remote_signer`, and `profile`.

### NostrIdentity Names

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

Link requests are fact operations with encrypted content:

- `type`: `nostr_identity_link_request`
- subject `i` tag: profile UUID
- `p` tag: invite pubkey
- `content`: NIP-44 ciphertext encrypted from the joining key to the invite key

The encrypted JSON contains `identity`, `admin_pubkey`, `invite_pubkey`,
`joining_pubkey`, `client_nonce`, `requested_at`, and optional `label`.
`requested_at` MUST equal event `created_at`; `joining_pubkey` MUST equal the
event signer; `invite_pubkey` MUST match the public `p` tag.
