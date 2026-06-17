# NIP-XX

## Fact Events

`draft` `optional`

Tag-only events for signed facts about one subject.

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

`content` MUST be empty.

## Index Tags

Single-character tags are index/reference tags, not facts.

Defined tags:

- `i`: subject and other searchable identifiers
- `p`: pubkeys
- `e`: event links
- `a`: addressable event links

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

## Trust

Consumers decide which authors, relays, snapshots, and predicates they trust.
