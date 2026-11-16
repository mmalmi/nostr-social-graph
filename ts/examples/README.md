# Social graph demo

Explore follow distances and mute relationships at [graph.iris.to](https://graph.iris.to).

## Data

Applications can read the signed graph, mobile projections and public profile
files directly from the [named Hashtree snapshot](../PUBLIC_GRAPH_SYNC.md#snapshot-and-local-data-rules).
The [checksum-pinned bootstrap](../README.md#published-bootstrap-snapshot) is also
available with the library. See the [server guide](../server/README.md) to run
your own crawler.

## Profile search index

The demo can search indexes with legacy 32-byte keys and versioned 33-byte keys.
New index writes use versioned CHK keys. Upgrade readers before publishing an updated index.

To point the examples search at a hashtree index, set `VITE_PROFILE_SEARCH_INDEX=nhash1qqsgm4ex4d4dxgz39hj6q7t7ax7u4k57gp2zkjuxtfga7wpw6dy6xpg9yqu6y09zecw9hzettkaulu928dt58ndt0h2exw6qg5kxyrprucz0cukym2c` (and optionally `VITE_BLOSSOM_SERVERS=url1,url2`).
Example published profile search index (2025-01-23): `nhash1qqsgm4ex4d4dxgz39hj6q7t7ax7u4k57gp2zkjuxtfga7wpw6dy6xpg9yqu6y09zecw9hzettkaulu928dt58ndt0h2exw6qg5kxyrprucz0cukym2c`.
To publish the profile search index to Blossom, run `BLOSSOM_NSEC=... pnpm publish-profile-index`.

[Back to the overview](../../README.md)
