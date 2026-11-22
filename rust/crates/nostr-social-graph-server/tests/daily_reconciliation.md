# Daily graph reconciliation measurement

Run the released reference and candidate binaries on the same isolated Linux host. The fixture creates a deterministic graph and locally signed follow and mute lists before starting each trial. The naming relay, two graph relays, Blossom store and reader all use loopback. Give every run a new output directory.

```sh
node daily_reconciliation.mjs --baseline /path/to/reference --candidate /path/to/candidate \
  --dependencies /path/to/ts/package.json --htree /path/to/current/htree \
  --legacy-htree /path/to/legacy/htree --output /path/to/paired-results
```

For the full trial, omit `--baseline`, add `--full-size` and provide another output directory. Use the pinned native CLIs and Node dependencies from the release qualification.

The paired input spans five layers of 1, 48, 192, 320 and 512 authors. Both binaries must visit every author and produce the same signed list heads and relationships. The fixture holds back part of the final layer and checks the service's public freshness state before allowing publication. It requires overlapping requests and a measured relay sweep speedup greater than 1.25× for the candidate.

The full synthetic input spans layers of 1, 420, 31,000, 164,000 and 123,000 authors. Follow and mute heads come from separate relays. Each relay independently delays EOSE using a synthetic per-layer latency distribution with nominal delays of 100, 300, 1,800, 350 and 300 milliseconds, deterministic jitter and an additional delay for the second relay. The inner layers carry heavier lists. The two-hour limit runs from crawler startup through an independent native reader resolving the signed name and downloading the complete published tree.

`receipt.json` records graph preparation time separately. It reports wall time to publication and cold read, time per layer, event-frame bytes, empty requests and relay responses, signed snapshot size, process CPU time, Linux crawler peak RSS and I/O. The semantic verifier compares each user's follow and mute edges, record timestamps, completeness and signed event IDs. Dataset generation and final independent verification run outside the timed interval. Measurements describe the synthetic workload and the selected runner.
