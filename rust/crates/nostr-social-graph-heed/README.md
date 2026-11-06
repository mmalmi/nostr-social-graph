# nostr-social-graph-heed

An LMDB-backed implementation of `nostr-social-graph`'s runtime interface.
Writes are transactional and immediately persistent; `flush` requests a durable
environment sync. Follow and mute event IDs, timestamps and completeness are
stored with the lists.

## Identity repair in 0.2.1

Valid 64-character hexadecimal public keys are case-insensitive. Other graph
identifiers keep their exact spelling. Queries, event authors, event tags and
root changes use the same canonicalization as the core library.

The first writable open of an older row-based database repairs its identity
indexes in one transaction. The migration reads both string-to-ID and
ID-to-string indexes, merges aliases through the core's list revision rules,
and rebuilds reverse edges and distances. Timestamped empty lists remain empty
when they win over an older alias. A version marker is committed with the
repaired data; subsequent opens do not repeat the migration. Legacy binary
snapshot databases continue to load through the core's binary reader.

Distinct identities assigned the same numeric ID, unknown authoritative node
references, and unresolved equal-time follow-list conflicts return an error.
Unresolved equal-time mute lists retain their union under the core's conservative
rule. No error replaces the database with an empty graph or a server snapshot.
An unsupported future identity schema is also rejected.

`replace_state` applies the same validation and normalization before writing.
If migration or replacement fails, including when LMDB runs out of map space,
the previous committed tables and revision metadata remain intact. Restore
capacity or resolve the input conflict before retrying. Numeric ID exhaustion
returns an error instead of overwriting an existing identity.

Upgrade every writer before opening an older database with 0.2.1; do not mix
older writers with the migrated format. Retain a backup for rollback. Existing
read-only export APIs remain read-only and do not perform a migration: upgrade
through the writable backend before relying on old row indexes for queries.

The migration needs working memory proportional to the stored graph and LMDB
space for the replacement transaction. It does not crawl relays, fetch a seed,
or discard personal graph history.
