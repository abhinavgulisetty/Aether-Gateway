# Local observability

After `python3 scripts/fetch-tools.py`, start the fixture and native collectors:

```bash
./scripts/demo.sh start
./scripts/observability.sh start
```

Prometheus listens on `127.0.0.1:9090`, Grafana on `127.0.0.1:3005`, and
node_exporter on `127.0.0.1:9200`. The gateway exposes `/metrics` on port 3000.
Grafana provisions its local Prometheus datasource and the dashboard at startup.
It disables plugin preinstallation/automatic updates and analytics reporting.
No external datasource or cloud account is involved.

You can instead import `grafana/dashboards/aether.json` into an existing local
Grafana. Create a Prometheus datasource with UID `aether-prometheus`, URL
`http://127.0.0.1:9090`, and scrape interval 2 seconds, matching `prometheus.yml`.
For native provisioning, run the supplied script from the repository; it sets
the Grafana home/data/provisioning paths explicitly. The dashboard provider's
relative dashboard path is resolved from that working directory.

## Metrics and boundaries

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `aether_requests_total` | Counter | route, status | Selected response-header status. 499 means cancellation before headers. |
| `aether_active_requests` | Gauge | route | Handler/body lifetime; includes policy rejection work briefly. |
| `aether_request_duration_seconds` | Histogram | route | Arrival through body EOF/error/drop, including backoff and downstream polling. |
| `aether_events_total` | Counter | route, event | Fixed events: auth, rate_limit, overload, retry, timeout, upstream_error, stream_error, cancelled, no_upstream, body_limit. |
| `aether_upstream_requests_total` | Counter | route, upstream | Reserved forwarding attempts, excluding active probes. |
| `aether_upstream_active_requests` | Gauge | route, upstream | Selected attempts through complete response body, error, or drop. |
| `aether_upstream_header_seconds` | Histogram | route, upstream | Attempt start to headers, transport error, or header deadline. |
| `aether_upstream_healthy` | Gauge | route, upstream | Last probe result, 0 or 1; startup is 0. |
| `aether_circuit_state` | Gauge | route, upstream | Closed 0, open 1, half-open 2. |
| `aether_circuit_transitions_total` | Counter | route, upstream | Every circuit state change. |
| `aether_connector_total` | Counter | result | Connector calls and successful/error/timeout outcomes; includes probes. Cancellation can leave a started call without an outcome. |
| `aether_connect_duration_seconds` | Histogram | none | Connector polling lifetime, covering DNS/TCP/TLS or cancellation. |
| `process_cpu_seconds_total` | Counter | scrape job/instance | Gateway user + system CPU from Linux /proc. |
| `process_resident_memory_bytes` | Gauge | scrape job/instance | Gateway RSS; not a heap allocation counter. |
| `process_open_fds` | Gauge | scrape job/instance | All open descriptors; not a connection-pool occupancy gauge. |
| `node_cpu_seconds_total` | Counter | host/cpu/mode | System CPU time from node_exporter. |
| `node_memory_MemAvailable_bytes` | Gauge | host | Host available memory. |

Routes and origins are fixed by configuration and capped at 64 routes and 32
origins per route. There are no raw URL, IP, key, trace-ID, or arbitrary error
labels. Status codes are finite HTTP protocol values. Common outcomes and all
fixed events initialize to zero; an idle rate still needs two scrapes before
Prometheus can compute a slope.

Counter events can overlap: a timed-out attempt may also be an upstream error,
and a response with status 200 can later time out while streaming. Do not sum
all event rates and call that the number of failed requests. A retry event
counts an actual subsequent selected attempt, not merely an intention to retry.

## Dashboard

The dashboard has four rows and 24 panels:

- Gateway Overview: RPS, success fraction, 5xx rate, active requests, p50/p95/p99.
- Upstream Health: active probe health, circuit state, per-origin traffic,
  p95 time to headers, active attempts, and circuit transitions.
- Performance: gateway CPU cores, RSS, connector activity, host CPU busy,
  host available memory, and open descriptors.
- Reliability: status rates, timeout/retry rates, policy and overload rejection
  rates, stream/transport/cancellation events, and backend recovery.

RPS comes from `sum(rate(aether_requests_total[$__rate_interval]))`. CPU cores
come from `rate(process_cpu_seconds_total{job="aether"}[$__rate_interval])`:
a value of 1 is one fully busy logical CPU. Host busy fraction is one minus the
average idle CPU rate. RSS and available memory are byte-valued gauges.

Latency uses classic histogram bucket rates:

```promql
histogram_quantile(0.99,
  sum by (le) (rate(aether_request_duration_seconds_bucket[1m])))
```

Actual panel queries use Grafana's dynamic rate interval. Datasource resolution
is set to the 2-second scrape interval. Buckets range from 0.5 ms to 10 seconds
plus infinity. Quantiles are bucket-interpolated estimates, especially coarse
below 0.5 ms or above 10 s; they are not exact per-request p99 observations.
Use oha output for client-side benchmark percentiles.

A high physical connector rate relative to a fixed workload can reveal socket
churn, but Hyper does not publish exact pool hits, misses, or live idle counts.
The fixture's `/__stats` TCP accept count provides independent reuse evidence.
Do not divide requests by connections and call the result a cache-hit ratio.

## Verified local rendering

[The dashboard screenshot](../docs/evidence/observability/dashboard.png) comes
from a real local Grafana browser render. Prometheus target snapshots, all 24
query evaluations, and a health failure/recovery scrape are retained in
`docs/evidence/observability/`. These prove a working local scrape/query path;
they do not constitute sustained monitoring or an alerting system. No alert
rules or notification integrations are configured.
