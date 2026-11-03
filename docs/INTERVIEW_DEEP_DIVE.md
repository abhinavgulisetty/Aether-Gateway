# Interview deep dive: explain the code and its evidence

## What problem does this solve?

A reverse proxy acts as a server to clients and a client to backends. Aether
provides a common routing and policy boundary for three local services. Unlike
a forward proxy, its origins come from trusted startup configuration, not an
arbitrary client-supplied destination. The original project already routed
prefixes, streamed HTTP bodies, authenticated API keys, and rate-limited IPs.
The new work adds bounded traffic management and observable failure behavior.

Walk a request through `proxy.rs`, `balance.rs`, and `body.rs`, then show the
actual observations in `docs/evidence/functional/results.json`. Avoid claiming
that every mature gateway feature is implemented.

## How does Tokio handle concurrency?

A request's async future advances until I/O is pending. Tokio can then run
other ready tasks rather than blocking an operating-system thread for each
socket. Hyper's connection tasks handle HTTP parsing and writing. Tokio's
multithreaded scheduler can move runnable tasks between workers, so shared
state uses Arc and synchronization. Async does not make CPU-heavy work free:
blocking inside a handler or holding a lock across an await would stall work.
Here the locks cover token arithmetic, selection, and breaker updates only.

## Why did pooling matter?

In commit `446d66b`, `forward_request` constructs a new client for each request.
Each request loses access to previous clients' idle sockets. The native fixture
counts TCP accepts independently of HTTP requests, providing a way to distinguish
socket churn from throughput claims. The staged revision had already fixed
this with one shared client, which the enhanced version preserves.

Keep-alive amortizes TCP establishment and, for HTTPS, TLS negotiation. It also
reduces ephemeral port churn. HTTP/1 generally needs one busy connection per
concurrent exchange; HTTP/2 multiplexes streams. Pool idle capacity is capped
per origin and expired using an explicit Tokio pool timer. The public client
API does not expose pool occupancy, so the dashboard does not invent a pool
hit percentage.

## Why TCP_NODELAY?

Nagle can postpone small writes while waiting for acknowledgements, interacting
badly with delayed acknowledgements. The historical saved experiment changed
accepted-socket TCP_NODELAY and saw about 42 ms p99 fall to about 2.76 ms. The
raw data supports the arithmetic of the earlier 15.8x claim, but the before/after
source and a fresh controlled Nagle-only rerun are not included. Explain this
as historical evidence with a plausible mechanism, not a newly proven causal
result from this implementation. Current code sets it on both sides.

## Why two balancing algorithms?

Round robin distributes request counts equally. Least-active counts requests
still waiting for headers or consuming their response bodies, rotating equal
counts fairly. With one backend delayed 80 ms, the recorded 90-request exercise
sent 30/30/30 using round robin and approximately 2/44/44 using least-active.
Read the saved elapsed times, which vary across runs. This demonstrates routing
behavior under uneven latency; it is not a universal throughput multiplier.

A key detail: the selected lease moves into the response body. Dropping it at
headers would make an endpoint with a long stream appear idle. Conversely,
counting a slow downstream against an upstream can bias routing away from a
healthy server. Production algorithms may need separate service-time and
client-delivery signals.

## Why both probes and circuit breakers?

A probe asks whether a health endpoint responds successfully. A breaker asks
whether application traffic is succeeding. The fixture can keep HEAD /health
healthy while returning 503 from application requests. Repeated failures open
the breaker; a cooldown admits exactly one half-open trial. A successful probe
alone cannot close it. A generation token prevents a late old request from
closing a newer circuit. Cancellation of a half-open trial reopens the breaker,
so it cannot get stuck indefinitely.

No hysteresis is implemented for active probes. One failed probe removes a
peer; one success restores its health flag. The breaker can still exclude it.
This favors fast local demonstrations and can flap under noisy conditions.

## Can retries make an outage worse?

Yes. Two attempts per incoming request can almost double backend work, including
work that succeeded but whose response was lost. Aether limits retries to empty
GET/HEAD/OPTIONS, caps attempts, prefers another eligible peer, and uses bounded
exponential delay within a total deadline. The permit spans backoff and all
attempts. It does not retry POST, stream bodies, or replay after downstream
headers. There is no jitter or shared retry budget; these are future controls
for correlated outages, not completed features.

## How do limits protect the process?

A token bucket permits a configured burst and replenishes sustained tokens
based on monotonic time. Its map has bounded capacity and idle expiry. The
admission semaphore bounds active proxied exchanges globally and rejects excess
work with 503 instead of storing a growing queue. A known oversized body gets
413; chunked bytes are checked while streaming. These mechanisms do not bound
idle downstream sockets or replace slow-client transport defenses.

Show the 48-request burst at a limit of eight: eight completions, forty 503s.
Show the three-IP demonstration at a two-entry bucket limit. These are local
observations, not distributed or production-scale guarantees.

## How do Prometheus and Grafana work here?

The Rust registry exposes counters, gauges, classic histogram buckets, and a
Linux process collector at /metrics. Native Prometheus scrapes it every two
seconds and also scrapes node_exporter for host CPU and memory. Grafana sends
PromQL queries to Prometheus. RPS comes from `rate(counter[window])`.

For p99, the dashboard evaluates:

```promql
histogram_quantile(0.99,
  sum by (le) (rate(aether_request_duration_seconds_bucket[1m])))
```

Keep `le` while combining histogram buckets. Compute a quantile after combining
buckets; do not average instances' p99 values. Quantiles are estimates inside
configured buckets. At very low traffic, tiny samples are not reliable tail
statistics. oha's client-side percentiles are separate observations and should
not equal server-side body-lifetime histograms exactly.

p99 exposes the slowest one percent that a median hides. Connection setup,
backend delays, retry amplification, scheduler contention, and downstream reads
all affect it. A fast 200 header followed by a failed stream illustrates why
status counts and latency alone are insufficient: inspect stream-error and
timeout events too.

Labels are configuration-derived route and origin plus bounded event/status
values. Raw request URLs, IP addresses, API keys, and arbitrary error strings
are never metric labels. Counts for common outcomes are initialized so healthy
zero-error charts show zero instead of an absent series.

## What was actually diagnosed?

The original source has a request-local client, excessive educational hot-path
printing, an always-on hardcoded rate limit, and no header sanitation or resource
deadlines. The staged baseline fixed pooling and socket configuration but had
unbounded IP state and no health-aware routing. The native benchmark captures
2xx RPS separately: interpreting the original's fast 429 responses as proxy
throughput would be a serious measurement error.

Read BENCHMARKS.md before quoting numbers. Use connector/socket evidence to
explain connection reuse, measured CPU to discuss bottlenecks, and the saved
raw distribution to discuss error rates. Shared-host CPU activity and a
single-threaded Node backend prevent a clean gateway scalability claim.

## What changes with multiple gateway instances?

Per-process bucket capacity, active counts, breaker state, and round-robin
cursors are independent. A user could receive a fresh burst at each instance.
One instance cannot infer load seen by another. A distributed design might use
shared policy storage, per-instance quotas, endpoint discovery, fleet-level
monitoring, and explicitly chosen consistency/availability tradeoffs. These are
future design changes, not claims of implemented distributed rate limiting.

For a production edge, additional work includes strict downstream connection
and idle limits, frontend TLS, protocol/security conformance, configuration
reload, secrets handling, robust retry budgets, hysteresis, broader profiling,
and sustained adversarial validation. This remains an explainable, measured
local systems prototype.
