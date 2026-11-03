# Historical optimization experiments

These notes refer to earlier builds. Current measurements are in
[../BENCHMARKS.md](../BENCHMARKS.md).

Exploratory runs used three five-second samples at 128 connections unless noted.
They identify direction; the five-by-ten-second result in `RESULTS.md` is the
claim-bearing measurement.

| Experiment | Outcome | Decision |
|---|---|---|
| Downstream `TCP_NODELAY` | Removed the 42 ms tail; 15.8x throughput at 32 connections | Keep |
| NGINX `worker_processes auto` | Raised the reference from ~21k to ~34k requests/s | Keep; corrected fairness issue |
| `std::sync::Mutex` for token bucket | Critical section never awaits; less scheduling machinery | Keep |
| Thin LTO, then fat LTO | Best short-run full profile rose from ~29k to ~32k requests/s | Keep fat LTO |
| Remove Tower trace layer | No measurable improvement | Revert |
| Remove duplicate proxy `info!` | Below noise; Tower already reports requests | Keep deletion |
| Merge two policy layers into handler | ~31.2k requests/s and fewer dispatches/files | Keep |
| Four Tokio workers | ~3% lower throughput, slightly lower p99 | Revert |
| One outer shared `Arc` | Best short run: 33.3k requests/s, 9.38 ms p99 | Keep |
| Single-pass header filtering | `HeaderMap` lacks in-place retain; rebuilding would allocate | Reject before benchmark |
| `target-cpu=native` | Did not reliably beat the portable build relative to NGINX | Do not default |

Raw exploratory outputs remain under `bench/results/` so unsuccessful trials are
auditable rather than hidden.
