# Decisions grounded in this implementation

| Decision | Reason and cost |
|---|---|
| Keep Axum + Tokio + Hyper | Existing streaming code works; a framework rewrite would complicate comparison without evidence that dispatch is the bottleneck. |
| Share one client | The original commit constructs a client per request. Native connection counters demonstrate that it opens essentially one backend socket per successful request. A shared client reuses warmed connections. |
| TCP_NODELAY on both sides | Already present in the staged baseline. Historical raw runs show a large tail-latency change; keep this setting but do not attribute that old improvement to the new feature work. |
| Stream bodies | Memory does not scale with an entire response's size; backpressure propagates through polling. The manual exercise transfers a 32 MiB body. This still uses bounded HTTP/TCP buffers. |
| Own permits in the response body | Releasing after headers undercounts slow requests, weakens admission, and makes least-active choose the wrong endpoint. RAII also releases on cancellation. |
| Fail-fast semaphore | Explicit overload response without an unbounded request queue. It may reject a burst that a small bounded queue could absorb. |
| Per-route endpoint state | Easy ownership and understandable isolation; duplicates probes for origins referenced by multiple routes. |
| Active probes plus passive breaker | Reachability does not establish application success. A health endpoint can return 200 while real requests return 503. The breaker covers that case. |
| Standard mutexes for small critical sections | No protected work awaits. Async mutex scheduling would add machinery; contention and expiry sweeps remain scaling limits. |
| Restricted retries | No request-body buffering or automatic POST replay. Even an idempotent request can repeat backend work after a lost response. Two attempts can almost double load. |
| No Hyper hidden retry | `retry_canceled_requests(false)` makes application attempts explicit, including reuse of stale pooled sockets. A failed POST is not silently retried. |
| Fixed route/origin labels | No user path, query, IP, key, arbitrary method, or error-message labels. Route and endpoint counts are validated. HTTP status has a finite protocol range. |
| Connection counters, not a fabricated pool-hit gauge | Hyper's public pooled client does not expose live occupancy/hits. Count actual connector activity and demonstrate reuse with backend socket accepts. |
| Native Prometheus, Grafana, node_exporter | Local processes and importable provisioning; no CI, deployment pipeline, cloud, or orchestration requirement. |
| Preserve the previous source snapshot | The starting staged source is different from Git HEAD. `bench/baseline` makes both baseline comparisons auditable and runnable. |
| Preserve truthful slow results | Report successful proxy RPS separately from rejection RPS, all raw runs, resource observations, and limitations. No promise to outrun NGINX. |

Round robin does less state inspection and provides equal *request counts*,
which is appropriate for similar-capacity endpoints with similar latency.
Least-active adapts to uneven latency and keeps slow endpoints from accumulating
the same request count. It does not know CPU utilization, backend queue length,
request cost, or other gateway instances' activity. HTTP/2 multiplexing makes
“active requests” more meaningful here than counting TCP connections. Neither
policy implements weights, sticky sessions, dynamic discovery, or distributed
coordination.

The retry backoff is exponential and bounded, without jitter or a global retry
budget. The admission permit spans backoff and every attempt, preventing an
unbounded retry queue. A correlated outage can still synchronize retries;
production extensions would add jitter and a retry budget before raising the
attempt count. Defaults favor a small explicit prototype.

Dependency constraints remain visible in Cargo.toml and exact versions in
Cargo.lock. Serde YAML is deprecated upstream; migration to a maintained parser
is a future maintenance task, not a measured performance claim. The project
keeps the existing format and rejects unknown configuration fields.

References used for implementation: [Hyper pool and retry builder](https://docs.rs/hyper-util/latest/hyper_util/client/legacy/struct.Builder.html),
[Hyper Rustls connector](https://docs.rs/hyper-rustls/latest/hyper_rustls/), and
[Prometheus histogram semantics](https://prometheus.io/docs/practices/histograms/).
