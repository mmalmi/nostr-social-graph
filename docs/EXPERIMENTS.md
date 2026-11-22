# Experiments

## Daily social graph reconciliation

The candidate fetched up to 4 independent author batches concurrently within each distance from the graph root. The reference fetched one batch at a time. On the paired workload, the relay sweep was **4.80× faster** and both binaries produced the same verified relationships and list IDs.

| Workload and binary | Prepare (s) | Relay sweep (s) | Startup to signed publication (s) | Startup to verified fresh read (s) | Crawler peak RSS (bytes) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Paired, reference | 1.350 | 10.619 | 10.809 | 21.015 | 19,537,920 |
| Paired, candidate | 1.355 | 2.214 | 2.521 | 12.719 | 20,480,000 |
| Full, candidate | 352.299 | 1,205.418 | 1,217.392 | 1,230.409 | 1,613,832,192 |

The paired workload included 1,073 authors, 6,736 follows and 172 mutes. The reader verified 257 follow-list IDs and 50 mute-list IDs. The full workload included 318,421 authors, 16,318,220 follows and 48,852 mutes. The reader verified 60,861 follow-list IDs and 10,856 mute-list IDs. Layers zero through four contained 1, 420, 31,000, 164,000, 123,000 authors, respectively. The full sweep issued 9,953 batches across the two relays. Both relays returned zero events for 1,243 of those batches.

Two synthetic relays supplied complementary signed follow and mute lists and independently delayed their end-of-results messages. Preparation generated the seed graph and signed test inputs before crawler startup. Relay sweep time spans the first graph request through the final relay response. Publication and fresh-read times both start at crawler launch. On the full run an independent reader took 13.017 seconds after publication to resolve the signed name, retrieve the tree and check its immutable content. A separate comparison then verified every relationship and list revision.

Times are rounded to the nearest millisecond. Each row represents one hosted run of that binary and workload. The two-hour acceptance limit applies to startup through the full signed read. Public relay timing and machine capacity can change the result. [Fixture and reproduction instructions](../rust/crates/nostr-social-graph-server/tests/daily_reconciliation.md) · [Build and correctness checks](https://github.com/mmalmi/nostr-social-graph/actions/runs/44979814952) · [Paired and full-size runs](https://github.com/mmalmi/nostr-social-graph/actions/runs/44980719065)
