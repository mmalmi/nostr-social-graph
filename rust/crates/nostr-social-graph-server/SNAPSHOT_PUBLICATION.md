# Named graph snapshots

The crawler can publish a complete public snapshot to `htree://<publisher-npub>/social-graph`. Readers pin the publisher separately from the pubkey that defines the graph’s scoring root. Clients with their own crawl keep control of whether to import a server snapshot.

## Publisher

Publishing defaults to off. Set these variables on the existing crawler to enable it.

| Variable | Value |
| --- | --- |
| `GRAPH_PUBLISH_HASHTREE` | `true` |
| `GRAPH_PUBLISH_NPUB` | Expected signing npub |
| `GRAPH_PUBLISH_HTREE_BIN` | Absolute path to the pinned native `htree` executable (qualified with 0.4.5) |
| `GRAPH_PUBLISH_BLOSSOM_URL` | Explicit HTTP(S) block upload server |
| `HTREE_CONFIG_DIR` | Dedicated native config directory containing `keys` and `config.toml` |
| `HTREE_DATA_DIR` | Existing dedicated writable directory for the temporary block store |
| `GRAPH_PUBLISH_INTERVAL_SECS` | Minimum time between captured checkpoints, default 900 seconds |

Provision the expected first signing key in `keys` and the name relays in `[nostr].relays` before startup. Restrict access to the signing key. The signing identity must match the configured npub. Invalid startup settings disable publication and log an error. Fix the settings and restart the crawler. Use `TMPDIR` on a filesystem with room to stage a full export, and provision the dedicated store for the encrypted chunks of one attempt. Normal completion or failure removes the temporary store. Uploads use the configured signer, which must be permitted to write to the target. The target must retain published blocks for as long as readers need the generation. The crawler’s existing data directory and relay configuration retain their usual roles.

The first successful graph and profile pass supplies the first generation. Later passes can publish once the interval elapses. Partial checkpoints and failed passes never advance the name. The checkpoint uses the crawler’s existing graph to encode both projections. Publication logs their sizes and the extra encoding time. One background worker stages the public files and uploads their encrypted content-addressed blocks before announcing the signed name. It releases serialized buffers after staging. A failed upload retries after 30 seconds using the previous staged files or a newer complete checkpoint. Older readers can decrypt the public-link tree using its 32-byte CHK. Crawl and HTTP service continue during upload. A completed pass can advance the manifest watermark when graph edges are unchanged.

## Reader contract

Resolve the signed name for the pinned publisher and directory, retain the full immutable root (hash and key), then fetch every file for one generation through that root. The manifest records version 1, the scoring pubkey in `root`, Unix seconds for `graph_completed_at`, `graph_since` and `generated_at`, plus the byte `size` and lowercase hex `sha256` for each data file. `graph_since` is the crawler’s overlapping resume cursor. `graph_completed_at` is the time of the successful graph pass. Compare checkpoint times for freshness.

| File | Contents |
| --- | --- |
| `social-graph.bin` | Full v3 graph for the ordinary notification consumer |
| `social-graph-v4.bin` | Full graph including follow-list and mute-list event revisions |
| `mobile.bin` | v4 projection for the scoring root, at most 4 MiB, with list completeness flags |
| `allowlist.txt` | Reachable pubkeys in graph distance order, one per line |
| `profile-data.json` | Public profile rows |
| `profile-index.json` | Matching public search index |
| `manifest.json` | Checkpoint metadata and digests of the six files above |

For the manifest’s scoring root, use `files["mobile.bin"]`. For another supported discovery root, look up its exact lowercase hex pubkey in `mobile_roots`. Each entry records `file` (`mobile-<root>.bin`), `size` and `sha256`. Fetch only that member from the same immutable tree, check its digest and the 4 MiB size limit, and keep the local graph when the requested projection is unavailable. Earlier manifests can omit `mobile_roots`.

A resolver stages files in an immutable directory. Verify the signed publisher, manifest, scoring root, sizes, digests and freshness before switching the local `current` symlink atomically. Preserve the previous usable generation across download or verification failures.

## Read-only HTTP service

Run `graph-snapshot-reader` with `GRAPH_SNAPSHOT_CURRENT` pointing to the resolver’s `current` symlink. Set `SOCIAL_GRAPH_ROOT` to the expected scoring pubkey. The default is the crawler’s usual public root. The reader listens on `BIND_ADDRESS` (default `127.0.0.1`) and `PORT` (default 3000). `ALLOW_ORIGIN` configures HTTP CORS.

The service checks for a new generation every five seconds (`GRAPH_SNAPSHOT_RELOAD_SECS`). It validates the manifest and loads the graph, profile rows and index before switching requests to that generation. The ordinary graph download returns the published v3 bytes. Graph size and distance parameters, the supported mobile roots, profile filters and distance-limited allowlists use the crawler’s query implementations. A failed refresh leaves the last verified generation serving. `/healthz` returns 503 before the first load and when the checkpoint is at least `GRAPH_SNAPSHOT_MAX_AGE_SECS` old (default 7200). Verified files remain available during staleness.
