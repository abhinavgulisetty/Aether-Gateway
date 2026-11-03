# Resume bullets with evidence

- Engineered a Rust API gateway with Tokio, Axum, Hyper, and Rustls, supporting
  pooled HTTP/HTTPS forwarding and streaming bodies; measured 23,666 successful
  requests/s at 32 concurrent connections across three 5-second local runs.
- Implemented health-aware round-robin/least-active routing, circuit breakers,
  bounded retries, and admission control; demonstrated 30 successful failover
  requests after stopping a backend and 40 overload rejections in a 48-request
  burst with an eight-request admission limit.
- Built Prometheus instrumentation and a 24-panel Grafana dashboard for latency,
  traffic, health, and resource use; diagnosed connection churn using backend
  counters, observing 355,881 requests over 7 new upstream sockets after
  warm-up versus one socket per request in the relaxed original data path.

Evidence: [benchmark methodology and raw records](../BENCHMARKS.md),
[functional observations](evidence/functional/results.json), and
[actual Grafana rendering](evidence/observability/dashboard.png).

These are local prototype results, with no claim of production traffic, cloud
scale, or business impact. The comparison does not isolate compiler flags or
individual feature costs. Do not replace the measured RPS with historical
15.8x or “NGINX parity” claims when describing this revision.
