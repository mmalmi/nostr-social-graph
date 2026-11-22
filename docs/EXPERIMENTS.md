# Experiments

## Profile index memory during graph publication

The candidate serializes one profile search record at a time and borrows its strings from the profile store. Both executables used the default allocator in separate cgroups on one Linux runner. The kernel recorded **3,044 MiB** peak memory for the previous implementation and its helpers and **2,245 MiB** for the candidate and its helpers. The crawler’s own peak RSS went from 2,587 to 1,757 MiB, a 32.1% decrease. The control was the previously qualified daily-crawl executable. The candidate integrated a newer graph library and lockfiles. Its change relative to that newer parent was confined to serialization. The measured differences apply to these two exact executables. Each completed five graph cycles, ten distance checkpoints and five signed publications. [Producer source](https://github.com/mmalmi/nostr-social-graph/blob/5cd34a0eafc991186d194f23a72f667702e106a7/rust/crates/nostr-social-graph-server/src/lib.rs) · [Paired run](https://github.com/mmalmi/nostr-social-graph/actions/runs/44988853513)

The frozen inputs held 189,959 profiles. The graph, profile rows and search index occupied 53.2, 25.6 and 25.1 MiB respectively. A profile sweep with no updates reproduced the original signed files. After one signed profile update, both executables produced byte-identical rows, search index, full graphs, mobile projections and allowlist. The final checkpoint contained 352,918 reachable users, 16,996,364 follows and 142,411 mutes. [Build and correctness checks](https://github.com/mmalmi/nostr-social-graph/actions/runs/44987924457)

| Phase | Previous total / anonymous / file (MiB) | Streaming total / anonymous / file (MiB) |
| --- | ---: | ---: |
| Verified idle | 2,559 / 2,379 / 170 | 1,751 / 1,573 / 169 |
| Graph crawl | 2,758 / 2,577 / 213 | 1,905 / 1,726 / 212 |
| Graph checkpoint and profile capture | 2,758 / 2,577 / 179 | 1,905 / 1,726 / 191 |
| Profile response through durable save | 2,848 / 2,525 / 349 | 2,048 / 1,735 / 300 |
| Publication or active helper | 3,032 / 2,525 / 519 | 2,216 / 1,747 / 518 |

Each column shows the sampled maxima of the dedicated cgroup’s total, anonymous and file-backed charges. These maxima can occur at different times. File-backed charges include cache, and phases can overlap. During verified idle both loops were waiting and no helper was running. The crawler’s peak proportional set size in that phase went from 2,387 to 1,581 MiB.

The 100 ms process sampler reads the parent and children separately. During helper startup one child reading resembled the parent and the sum exceeded the kernel’s peak. The kernel cgroup counters supply the whole-service measurements above.

Median time from the final profile relay response through durable save went from 0.35 s to 0.20 s. Median time from submission to a signed publication went from 2.35 s to 2.30 s. A fresh reader resolved and downloaded the final tree in 13.16 s and 13.18 s respectively. [Reproduction workflow](https://github.com/mmalmi/nostr-social-graph/blob/e3af1b7f175ca78c46f057e5f594daf0b361cc89/.github/workflows/graph-server-recovery.yml)

An earlier producer binary peaked at 1,610 MiB with a single profile and 2,691 MiB with 189,959 profiles. Those separate runs used the same graph with different profile loads and cycle intervals. The paired runner above held both fixed. [Earlier single-profile run](https://github.com/mmalmi/nostr-social-graph/actions/runs/44159503711) · [Representative baseline](https://github.com/mmalmi/nostr-social-graph/actions/runs/44988174856)

### Reversed execution order

A second runner executed the streaming build first. The previous and streaming implementations recorded whole-service kernel peaks of 2,979 and 2,267 MiB, respectively. Their crawler RSS peaks were 2,482 and 1,813 MiB. Across verified idle periods the crawler’s maximum PSS was 2,332 and 1,631 MiB respectively.

The second sampler checked each helper’s process generation, executable and group membership before and after reading its memory. The largest individual Hashtree helpers used 152 and 157 MiB PSS in the previous and streaming builds respectively. The sampler recorded zero complete `df` helper reads. In 9 samples a process changed, disappeared or could not be identified. Those samples retained their kernel group measurements and omitted the process sum. All seven output files matched the first comparison. [Reversed-order verification](https://github.com/mmalmi/nostr-social-graph/actions/runs/44989597450)

### Complete daily crawl

The same candidate executable visited 318,421 synthetic authors across five distances, completed its relay sweep in 20 minutes 6 seconds and published a signed tree at 20 minutes 17 seconds. An independent reader verified it at 20 minutes 30 seconds after crawler startup. All 16,318,220 follow edges, 48,852 mute edges and 71,717 signed list IDs matched the earlier full-depth run. Preparing the signed inputs took a separate 4 minutes 40 seconds. [Complete daily run](https://github.com/mmalmi/nostr-social-graph/actions/runs/44988867948) · [Earlier daily crawl measurements](https://github.com/mmalmi/nostr-social-graph/blob/1cc8fb09066322b64fa00c560515cdfe993278ce/docs/EXPERIMENTS.md)
