# Historical benchmark results (2026-08-22)

Recorded 2026-08-22 on Linux 7.0, Docker 29.7.2, an Intel Core i5-10300H
(4 cores / 8 threads), and 14 GiB RAM. All containers and the load generator
shared the host. The final table uses the median of five 10-second oha runs at
128 connections; every response was HTTP 200.

## Final portable build

NGINX uses `worker_processes auto`; Aether uses portable fat-LTO release code.

| Target | Requests/s | p50 | p95 | p99 |
|---|---:|---:|---:|---:|
| Direct upstream | 52,701 | 1.96 ms | 5.85 ms | 8.89 ms |
| NGINX reverse proxy | 29,886 | 3.69 ms | 9.62 ms | 13.69 ms |
| Aether, policy disabled | 29,229 | 3.97 ms | 8.20 ms | 11.37 ms |
| Aether, auth + rate limit | 29,084 | 4.05 ms | 8.11 ms | 11.26 ms |

Aether-full is within 2.7% of NGINX throughput, with 15.7% lower p95 and 17.8%
lower p99. Authentication plus rate limiting cost 0.5% versus Aether's
policy-disabled profile.

Raw JSON and environment: [`results/20260822T064644Z`](results/20260822T064644Z).

## Root-cause result

At 32 connections, setting `TCP_NODELAY` only on upstream sockets left Aether at
1,740 requests/s and 42.11 ms p99. Axum 0.8 had removed its old server-level
setting, so accepted downstream sockets still used Nagle's algorithm. Applying
`ListenerExt::tap_io` to those sockets produced 27,506 requests/s and 2.76 ms
p99: 15.8x throughput and 93.4% lower tail latency.

Before: [`results/20260822T055652Z`](results/20260822T055652Z). After:
[`results/20260822T062054Z`](results/20260822T062054Z).

## Host-specific build

`RUSTFLAGS="-C target-cpu=native"` was tested separately. Its five-run result
was 30,584 requests/s and 11.22 ms p99 versus NGINX at 32,687 requests/s and
12.38 ms p99. It did not close the throughput gap reliably, so the default build
remains portable. Raw data: [`results/20260822T065305Z`](results/20260822T065305Z).

## Limits

These are local comparative measurements, not vendor-neutral gateway rankings.
The workload uses one small static response, one route, HTTP/1, one client IP,
no TLS, and sequential targets. Target order alternates between runs to reduce
bias, but shared-host load and thermal variation remain visible.

## Current reproduction

These measurements describe the earlier container-based experiment, not the
current native implementation. The former launcher has been replaced. Use
[../BENCHMARKS.md](../BENCHMARKS.md) and `./bench/run.sh` for the current local
comparison. Do not attribute this historical table to the enhanced gateway.
