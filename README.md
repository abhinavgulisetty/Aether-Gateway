# Aether Gateway

Aether Gateway is a fast, asynchronous HTTP reverse proxy and API gateway written in Rust. Built on top of the Axum web framework, the Tokio async runtime, and Hyper, it acts as a centralized entry point for routing and managing HTTP traffic to upstream services.

## What the Project Does

Aether Gateway handles incoming HTTP requests and proxies them to the appropriate backend services based on rules defined in a centralized YAML configuration file. The user connecting to the gateway remains completely unaware of the upstream infrastructure.

Current features include:
- **Dynamic Routing:** Configuration as code via `config.yaml`. Routes traffic based on matching path prefixes to specific upstream URLs.
- **Prefix Stripping:** Optionally strips the matching path prefix before forwarding the request to the upstream server, enabling clean internal routing.
- **Query Parameter Support:** Automatically preserves and forwards query parameters to upstream destinations.
- **Rate Limiting:** Protects backend services using a Token Bucket algorithm, applying configurable burst capacity and sustained token refill rates to throttle requests on a per-IP basis.
- **Authentication:** Enforces security by requiring clients to provide a valid API key via the `x-api-key` HTTP header. 

## Implementations that Remain

While the core proxying and middleware functionality is established, several key optimizations and features are pending:

- **HTTP Client Connection Pooling:** Currently, a new Hyper HTTP client is instantiated for every incoming request. This needs to be refactored so that a single client is instantiated once and shared across all requests to efficiently pool and reuse connections.
- **Structured Logging and Tracing:** Diagnostic output relies on basic `println!` and `eprintln!` macros. This needs to be upgraded to a robust, structured tracing system for production-ready observability.
- **Configuration Hot-Reloading:** Any modifications to the `config.yaml` file (such as adding routes or changing rate limits) currently require a full server restart. Dynamic, seamless hot-reloading of the configuration is planned to prevent downtime.
