# Native performance measurements

The current comparison runs native local processes, without containers or
cloud infrastructure. This document separates successful proxy throughput,
rejection throughput, functional behavior, and historical evidence.

## Targets and fairness

| Target | Source / behavior |
|---|---|
| direct | The same Node HTTP fixture reached directly. |
| original | Unmodified commit `446d66b`, release build. Its hardcoded per-IP bucket ignores configuration; most responses are 429, not proxied work. |
| original-unlimited | The same commit with only `TokenBucket::new(10.0, 2.0)` changed to `(1e12, 1e12)`. This permits a successful data-path comparison; it is explicitly not the unmodified original. |
| previous | Staged source found at task start, preserved in `bench/baseline`; already pooled and TCP_NODELAY-enabled. |
| enhanced | Current gateway, including metrics, health probes, admission, timeout, circuit, and retry machinery. |
| nginx | Native NGINX with `worker_processes auto`, keepalive 32 per worker, HTTP/1.1 backend, access logs disabled. |

All targets use a 256-byte plain HTTP response from `scripts/upstream.mjs`.
Authentication and rate limiting are disabled for previous/enhanced, while the
original's limitation is reported explicitly. Baseline original prints are
redirected to /dev/null without editing the source. NGINX retains default
response buffering, while Aether streams. This is comparable small-response
proxy work, not identical implementations or a TLS/large-body comparison.

Release profiles differ: original is its original Cargo release profile;
previous/enhanced use portable fat LTO and one codegen unit. Dependencies are
locked per source. Consequently, the original-to-enhanced comparison measures
an implementation change set, not the isolated effect of pooling or compiler
flags. The original-unlimited patch and binary hashes are saved with each run.

NGINX keeps 32 idle connections **per worker**, while enhanced Aether's default
cap is 32 **per origin across the process**. The comparison retains NGINX's
default request-count retirement, which closes a keep-alive connection after
1,000 requests. This explains some observed NGINX socket turnover and is not
evidence of a pooling defect. See the [NGINX upstream documentation](https://nginx.org/en/docs/http/ngx_http_upstream_module.html#keepalive_requests).
At concurrency 128, Aether can also reopen sockets after its smaller idle cap
is exceeded; resource bounds trade some reuse for a bounded idle pool.

## Reproduction

```bash
python3 scripts/fetch-tools.py --bench-only
DURATION=5s WARMUP=2s RUNS=3 CONCURRENCIES=1,32,128 ./bench/run.sh
```

The harness builds all artifacts before measuring. It gives each workload a
warm-up, alternates forward/reverse target order across repeats, waits for
outstanding requests at each deadline, records raw oha JSON, and terminates its
own processes. It uses ports 19201–19206. Do not run two copies simultaneously.
To use externally installed tools, set `OHA`, `NGINX`, or `AETHER_TOOLS_DIR`.

Record CPU as cumulative /proc user+system seconds divided by elapsed wall
time, where 100% is one logical core. NGINX includes master and workers.
Upstream CPU is recorded separately; load-generator CPU uses child resource
usage. RSS is sampled every 100 ms and includes NGINX workers; their shared
pages are counted repeatedly, so summed RSS is not unique physical memory.
New upstream accepts are measured through one persistent diagnostic connection,
excluding warm-up and avoiding a new diagnostic socket on every sample. Probe
requests are excluded from fixture application counts but may create sockets.

Raw results, environment details, source-diff hashes, binary hashes, status/error
counts, resource samples, and summaries are under
[bench/results/native-20261103](bench/results/native-20261103).

An interrupted preliminary run in
[bench/results/native-invalid-cache](bench/results/native-invalid-cache/INVALID.md)
is explicitly invalid: restored Git timestamps allowed a cached relaxed-limiter
binary to be reused for the original target. The corrected harness refreshes
source mtimes and checks distinct binary hashes and original 429 behavior before
measurement. No measurements from that run support current claims.

The complete preliminary comparison in `bench/results/native-before-uri-guard`
is valid for its earlier build and retained separately. The final comparison
was repeated after adding a safe 414 response for URI expansion overflow.
Samples from those two matrices are never pooled.

Hardware profiling with `perf` was unavailable (`perf_event_paranoid=4`). No
flamegraph, instruction count, or attribution of CPU samples is claimed.

## Current measured results

Recorded 2026-11-03 on a shared Intel Core i5-10300H workstation (4 cores,
8 hardware threads, approximately 15 GiB RAM), Rust 1.93.0, Node 24.13.1,
oha 1.16.0, and native NGINX 1.31.8. Each cell is the median of three 5-second
runs after a 2-second warm-up per workload. Logging inherited RUST_LOG=warn;
RUSTFLAGS was unset. Exact environment and source/binary hashes are saved.

| Target | Concurrent | 2xx RPS | p50 ms | p95 ms | p99 ms | CPU % | Peak RSS MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| direct | 1 | 20,520 | 0.044 | 0.059 | 0.069 | 88.4 | 81.2 |
| original-unlimited | 1 | 6,502 | 0.146 | 0.189 | 0.239 | 75.4 | 4.3 |
| previous | 1 | 10,665 | 0.092 | 0.109 | 0.146 | 48.2 | 4.4 |
| enhanced | 1 | 10,096 | 0.094 | 0.121 | 0.155 | 50.5 | 6.4 |
| nginx | 1 | 12,216 | 0.082 | 0.094 | 0.136 | 40.2 | 30.0 |
| direct | 32 | 27,828 | 1.086 | 1.374 | 2.137 | 99.1 | 102.9 |
| original-unlimited | 32 | 9,598 | 3.421 | 3.752 | 3.974 | 141.1 | 12.2 |
| previous | 32 | 24,499 | 1.252 | 2.254 | 2.612 | 126.8 | 9.9 |
| enhanced | 32 | 23,666 | 1.294 | 2.307 | 2.699 | 148.9 | 12.9 |
| nginx | 32 | 25,892 | 1.163 | 2.027 | 2.538 | 83.4 | 30.7 |
| direct | 128 | 28,004 | 4.425 | 5.173 | 5.825 | 99.1 | 107.6 |
| original-unlimited | 128 | 9,376 | 13.937 | 15.051 | 16.081 | 142.4 | 25.4 |
| previous | 128 | 23,289 | 5.487 | 5.964 | 10.162 | 130.2 | 19.9 |
| enhanced | 128 | 22,366 | 5.549 | 6.204 | 10.974 | 150.5 | 25.0 |
| nginx | 128 | 23,737 | 5.269 | 5.940 | 8.034 | 93.8 | 30.7 |

All measured successful-proxy targets returned only HTTP 200 with no oha
transport errors. The unmodified original, omitted from this successful-work
table, accepted about two requests/s and rejected the rest with 429. Its much
higher all-response RPS and mixed-response percentiles are in the raw summary;
they are not successful proxy throughput.

At concurrency 32, enhanced Aether's 23,666 RPS is 8.6% below
NGINX and 3.4% below the already pooled previous baseline.
This measures the complete resilience/observability revision, not the isolated
cost of any single feature. At concurrency 128, its median is
22,366 RPS versus NGINX's 23,737;
p99 is 10.974 versus 8.034 ms. NGINX remains faster
in this workload; no result was selected to imply otherwise.

Across the three concurrency-32 measured enhanced runs, the fixture handled
355,881 application requests and accepted 7 new upstream TCP connections
after warm-up. The relaxed original opened 144,316 sockets for
144,316 requests. This directly demonstrates the original socket
churn and warmed-pool reuse; it is not a public Hyper pool-hit metric.

The fixture consumed roughly 99% of one CPU during enhanced
concurrency-32 traffic and 99% during direct concurrency-128 traffic.
Increasing concurrency did not increase successful throughput proportionally.
These observations support a single-event-loop backend bottleneck in this setup,
not a proven gateway maximum. Enhanced gateway CPU was
148.9% (one core = 100%) versus NGINX's 83.4%.
The enhanced gateway's sampled RSS median was 12.9 MiB at
concurrency 32 and 25.0 MiB at 128. Shared-host noise,
compiler-profile differences, NGINX buffering, and pool defaults limit attribution.

## Historical claim audit

The saved August data reproduces the earlier arithmetic:

| Earlier experiment | Full-policy Aether RPS | p99 ms |
|---|---:|---:|
| `20260822T055652Z` | 1,740.24 | 42.109 |
| `20260822T062054Z` | 27,506.21 | 2.757 |

The ratio is 15.81×. The historical experiment notes attribute this to accepted
socket TCP_NODELAY; a roughly 42 ms tail is consistent with delayed-ACK/Nagle
interaction. These artifacts are not a controlled new causal experiment: exact
before/after source snapshots were not preserved with those runs.

The historical five-run `20260822T064644Z` medians are 29,084.24 RPS for
policy-enabled Aether and 29,886.05 RPS for NGINX, a 2.68% throughput gap;
p99 is 11.256 versus 13.691 ms. These are earlier container/shared-host results
and cannot be transferred to the enhanced native build. All historical raw
files and unsuccessful experiments remain available in `bench/results/`.

## Functional performance and fault observations

The separate targeted local exercise uses 90 requests, six workers, three
backends, and one backend delayed by 80 ms. Round robin produces 30/30/30 request
counts; least-active sends only a few to the slow peer and most to the other
two. The recorded elapsed times are in
[docs/evidence/functional/results.json](docs/evidence/functional/results.json).
This is a small behavior experiment, not an oha percentile benchmark.

Stopping a backend followed by a 350 ms observation delay left 30 successful
requests and no traffic to that peer. After restart and probes, 12 requests
split 4/4/4. Header delay exercises produced a 504 after two 150 ms attempts;
streaming exercised both a complete 32 MiB response and truncation at a 1.5 s
deadline. These scenarios complement the steady-state proxy comparison.

## Interpretation limits

This is a shared Linux workstation with desktop/editor/browser activity, not
an isolated benchmarking host. Targets and load generator compete for CPU and
memory. Node is a single event-loop upstream and can cap proxy throughput.
No CPU affinity, thermal control, confidence interval, or fleet-scale result is
claimed. Three five-second repeats establish a local median, not statistical
proof of a small difference. Closed-loop concurrency can underrepresent queueing
latency under a fixed arrival rate; use oha `-q` with latency correction for a
separate arrival-rate experiment.

Median p50/p95/p99 across runs are summaries of per-run percentiles, not a
percentile of merged samples. Histograms in Grafana are bucket estimates of
server-side body lifetimes; oha percentiles measure the client experience.
Error/rejection responses can be faster than successful responses. Always
inspect 2xx counts, status distribution, and transport errors before calling an
RPS number proxy throughput. Connection counts and CPU measurements support
specific explanations; they do not substitute for a CPU flamegraph.
