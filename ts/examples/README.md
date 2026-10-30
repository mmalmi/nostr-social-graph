# Social graph demo

Explore follow distances and mute relationships at [graph.iris.to](https://graph.iris.to).

## Data

The public API provides [graph snapshots](https://graph-api.iris.to/social-graph?maxBytes=2000000)
and [profile data](https://graph-api.iris.to/profile-data?maxBytes=2000000&noPictures=true).
See the [server guide](../server/README.md) to run your own service, or use the
[checksum-pinned bootstrap snapshot](../README.md#published-bootstrap-snapshot).

## Profile search index

To point the examples search at a hashtree index, set `VITE_PROFILE_SEARCH_INDEX=nhash1qqsgm4ex4d4dxgz39hj6q7t7ax7u4k57gp2zkjuxtfga7wpw6dy6xpg9yqu6y09zecw9hzettkaulu928dt58ndt0h2exw6qg5kxyrprucz0cukym2c` (and optionally `VITE_BLOSSOM_SERVERS=url1,url2`).
Example published profile search index (2025-01-23): `nhash1qqsgm4ex4d4dxgz39hj6q7t7ax7u4k57gp2zkjuxtfga7wpw6dy6xpg9yqu6y09zecw9hzettkaulu928dt58ndt0h2exw6qg5kxyrprucz0cukym2c`.
To publish the profile search index to Blossom, run `BLOSSOM_NSEC=... pnpm publish-profile-index`.

[Back to the overview](../../README.md)
