# Failure behavior and observed evidence

Run `cargo build --release --locked && python3 scripts/verify.py`. This is a
small, standalone sequence of local HTTP demonstrations, not an automated test
framework. It starts three isolated upstreams and two gateways on ports
19101–19103 and 19300–19301, writes logs/observations, and stops its processes.
The committed execution record is [evidence/functional/results.json](evidence/functional/results.json).
It intentionally raises on an unexpected result; check the printed evidence
path and logs rather than claiming an unsuccessful run passed.

| Trigger | Implementation behavior | Exercised outcome |
|---|---|---|
| Missing API key | 401; bounded auth event | Missing key rejected; valid key accepted. |
| Empty token bucket | 429 + Retry-After | Burst of two produced 200, 200, 429. |
| New IP at bucket capacity | Reject without allocating another entry | Third loopback IP rejected at capacity two, then accepted after idle expiry. |
| Backend process stops | Active probes remove it | 30 subsequent round-robin requests succeeded; none reached the stopped peer. |
| Backend restarts | Successful probe restores eligibility | Next 12 round-robin requests split 4/4/4. |
| Health succeeds, application returns 503 | Passive failures open circuit | Open state scraped; request excluded; cooldown trial succeeded after repair. |
| Bodyless safe request fails on one peer | Retry another eligible peer | 12 requests returned 200 despite one backend configured to return 503. |
| POST receives 503 | No replay | Exactly one backend attempt observed. |
| Slow response headers | Per-attempt timeout; bounded retry | 504 in about 0.31 s with two 150 ms attempts and 10 ms backoff. |
| Slow streamed body | Total deadline truncates body | Stream stopped before completion at 1.5 s; header status was already 200. |
| 48 simultaneous requests, admission limit 8 | No queued overflow | Eight requests completed; forty received 503. |
| Oversized Content-Length | Reject before forwarding | 413 from headers alone, without transmitting the oversized body. |
| Origin plus path exceeds URI capacity | Return a client error without panicking | 414 for a long valid incoming URI; process remained responsive. |
| Large valid response | Stream frames without full-body collection | 32 MiB response transferred successfully. |
| Client disconnect during stream | RAII drops active counters and permit | Cancellation event observed and active count returned to zero. |
| SIGTERM during finite stream | Drain before exit | All 1,310,720 bytes received; gateway exited with status zero. |

The upstream fixture records `accepted` TCP connections independently of
application request counts. Sixty sequential round-robin requests produced
20/20/20 distribution and only a handful of new sockets (including separate
stats requests). The machine-readable run gives the precise count. Health
probes are separate HEAD requests, so they do not inflate application counts.

## Protocol observations

The separate [protocol record](evidence/protocols/results.json) confirms 100
successful cleartext HTTP/2 requests to a local HTTP/1 backend. The HTTPS check
proxied Cloudflare's public trace endpoint with certificate verification,
observing HTTP/2 over TLS 1.3 upstream. Only protocol fields are stored; the
remote trace's client IP is omitted. This internet-dependent check is separate
from the fully local HTTP/1 workload. Health uses HEAD / because that service's
trace endpoint answers GET successfully but returns 404 to HEAD.

## Additional local failure exercises

Start `./scripts/demo.sh start` and `./scripts/observability.sh start`; put
`.local/tools` on PATH for `oha`. Grafana is at localhost:3005 and Prometheus at
localhost:9090. Keep traffic running in another terminal:

```bash
./scripts/demo.sh traffic
./scripts/demo.sh kill-upstream 9102
./scripts/demo.sh recover-upstream 9102
./scripts/demo.sh slow 9102
./scripts/demo.sh reset
```

The default limiter permits 50 sustained requests/s with a burst of 100.
Saturation workloads will deliberately return 429. To compare routing policies,
use the `/` round-robin route and `/api/` least-active route. Edit the local
configuration to disable limiting for a routing throughput experiment and
restart the gateway; never count rejected responses as successful proxy work.

To trip a circuit while health remains good:

```bash
curl -X POST http://127.0.0.1:9102/__control -d '{"status":503}'
# Generate traffic for several seconds; inspect aether_circuit_state.
./scripts/demo.sh reset
```

Prometheus failure/recovery API snapshots are retained under
[evidence/observability](evidence/observability). Dashboard quantiles estimate
bucketed observations and include downstream body delivery time; header latency
has its own histogram. A status chart alone misses failures after headers.

## Explicit limits

- A failed request may have reached the backend before the gateway sees an
  error. There is no exactly-once guarantee, even for a safe method.
- No retries after response headers, no replayed bodies, no hedged requests,
  no global retry budget, and no jitter. Full peer failures can yield 503 before
  the configured attempt count is exhausted.
- Health is one-probe sensitive with no rise/fall hysteresis. A short failure
  can flap state; a failure between probes can hit one or more requests.
- Upstream 5xx are breaker failures. Application semantics may require a more
  selective policy. Client cancellation does not ordinarily blame a backend,
  but a cancelled half-open trial reopens its circuit to avoid wedging it.
- Streaming request bytes are limited while polled. A chunked upload exceeding
  the limit fails its upstream exchange; it does not reliably become a 413
  after forwarding has started. This is distinct from a known-length 413.
- Response-header timeouts include upload time. The total body deadline is
  enforced when polled; it is not a strict bound on a socket blocked by a
  downstream that never reads. Idle downstream sockets are not globally capped.
- No inbound TLS termination, WebSocket upgrades, CONNECT tunnels, custom trust
  stores, mTLS, rate-limit sharing, configuration reload, or service discovery.
- HTTP/2 support does not establish protocol conformance certification. HTTPS
  and h2 checks, when performed, are recorded separately from HTTP/1 benchmarks.
- Drain is bounded. Infinite streams can be terminated at the shutdown deadline.
- The loopback fixture has unauthenticated diagnostic control endpoints; it is
  a demonstration utility, not an application to expose on a public interface.
