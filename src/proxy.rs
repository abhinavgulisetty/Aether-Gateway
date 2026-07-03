// =============================================================================
// AETHER-GATEWAY: Reverse Proxy Module
// =============================================================================
//
// EVOLUTION OF THIS FILE:
// - Phase 1: Hardcoded forwarding to httpbin.org (the "dumb forwarder")
// - Phase 2: Dynamic routing based on config.yaml (current)
//
// This module takes an incoming HTTP request from the user and FORWARDS it to
// an upstream server. It then returns the upstream's response back to the user.
// The user never knows the upstream exists.
//
// ANALOGY: You're a receptionist. Someone asks you a question. You don't know
// the answer, so you call the expert (upstream server), relay their answer back,
// and the person thinks YOU answered.
//
// Flow:  User -> Aether (this code) -> upstream server -> Aether -> User
// =============================================================================

// ===== RUST CONCEPT: Module imports (Phase 0) =====
// `use` is like Python's `from module import thing` or JS's `import { thing } from 'module'`.
//
// Rust's module system is EXPLICIT -- you must import every type and function
// you want to use. Nothing is available by default except a small "prelude"
// (basic types like Option, Result, Vec, String).
//
// The `::` separator is like `.` in Python/JS. So `axum::Router` means
// "the Router type from the axum crate".
//
// `crate::` refers to the ROOT of your own project. If you used `mod proxy;` in
// main.rs, then proxy.rs becomes a module at `crate::proxy`.
use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
// ===== RUST CONCEPT: std::sync::Arc (Phase 2) =====
// Arc = "Atomic Reference Counted pointer"
//
// The problem: In an async web server, multiple requests are handled
// CONCURRENTLY (by different Tokio tasks). They all need to read the config.
// But Rust's ownership rules say: one owner, one copy.
//
// Arc solves this: it wraps a value and lets you clone CHEAP references to it.
// Each clone increments a counter. When the last reference is dropped, the
// value is freed. Like Python's reference counting, but explicit.
//
//   let config = Arc::new(my_config);
//   let config2 = Arc::clone(&config);  // Both point to the same data
//   // config and config2 can be used in different threads safely
//
// Key difference from Python: In Python, all objects are reference-counted
// automatically. In Rust, you opt in by using Arc when you NEED sharing.
use std::sync::Arc;

use crate::config::GatewayConfig;

// ===== RUST CONCEPT: pub (public visibility) -- Phase 1 =====
// By default, everything in Rust is PRIVATE -- only accessible within the same module.
// `pub` makes it accessible to other modules (like main.rs).
//
// Python: Everything is public by default. `_prefix` is a convention, not enforced.
// JS: Uses `export` keyword.
// Rust: Uses `pub` keyword. The compiler ENFORCES privacy.
//
// ===== RUST CONCEPT: Ownership & Borrowing -- Phase 1 (THE BIG ONE) =====
// `request: Request` -- we TAKE OWNERSHIP of the request.
//
// In Python/JS, when you pass an object to a function, you pass a REFERENCE.
// Both the caller and callee can use the object. Memory is garbage-collected.
//
// In Rust, passing a value MOVES it. The caller can NO LONGER USE IT.
// This is how Rust avoids garbage collection: there's exactly ONE owner,
// and when the owner goes out of scope, the memory is freed.
//
//   Example:
//     let request = get_request();
//     forward_request(request);   // `request` is MOVED into the function
//     println!("{:?}", request);  // COMPILE ERROR! `request` was moved.
//
// If you need to SHARE without moving, you use references:
//   &request  = immutable borrow (read-only, like a const reference)
//   &mut request = mutable borrow (read-write, exclusive: only one at a time)
//
// Here, we take ownership because we consume the request -- we tear it apart
// (extract method, URI, headers, body) and build a NEW request for the upstream.
//
// ===== RUST CONCEPT: Axum State Extractor (Phase 2) =====
// Axum uses "extractors" to pull data from requests. State<T> extracts
// shared application state that you registered with .with_state().
//
// Think of it as dependency injection:
//   Express.js: app.locals.config -> req.app.locals.config
//   Flask: app.config -> current_app.config
//   Axum: .with_state(config) -> State(config) in handler parameters
//
// Axum calls this function with the right arguments automatically by looking
// at the parameter types. This is compile-time checked -- if you ask for
// State<String> but registered State<i32>, it won't compile.
//
// ===== RUST CONCEPT: Function signatures (Phase 0) =====
// `async fn name() -> ReturnType { ... }`
//
// `Response` is an HTTP response type. In Phase 0, we used `impl IntoResponse`
// (meaning "any type that can become a response"). Now we use the concrete
// `Response` type directly because we need to return it from different code paths
// (match arms, if/else branches) which works better with a concrete type.
pub async fn forward_request(
    State(config): State<Arc<GatewayConfig>>,
    request: Request,
) -> Response {
    // ===== RUST CONCEPT: String vs &str (Phase 1) =====
    // This is one of Rust's most confusing concepts for newcomers. There are TWO string types:
    //
    // &str (string slice):
    //   - A BORROWED reference to string data somewhere in memory
    //   - Read-only, cannot grow or shrink
    //   - Like a Python `str` or JS `string` (but borrowed, not owned)
    //   - "hello" is a &str literal baked into the compiled binary
    //
    // String (owned string):
    //   - Heap-allocated, growable, mutable
    //   - YOU own the memory and can modify it
    //   - Like a Python `str` that you can concatenate to
    //   - Created by String::from("hello") or "hello".to_string() or format!()
    //
    // .path() returns &str because the URI already exists in memory -- no need to allocate.
    // .to_string() converts &str -> String (allocates new memory, copies the data).
    // We need an owned String here because we use it later after `request` is consumed.
    let path = request.uri().path().to_string();

    // ===== RUST CONCEPT: Option<T> and method chaining (Phase 1) =====
    // Rust has NO null, nil, None (in the Python sense), or undefined.
    // Instead, it uses Option<T>:
    //   Some(value) = there IS a value
    //   None = there is NO value
    //
    // .query() returns Option<&str> -- there might not be query parameters.
    //
    // .map() transforms Some(x) into Some(f(x)), leaves None as None.
    //   Python equivalent: `f"?{q}" if q is not None else ""`
    //
    // .unwrap_or_default() returns the inner value or "" (the default for String).
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();

    // ===== RUST CONCEPT: if let -- pattern matching shorthand (Phase 2) =====
    // `if let Some(route) = expression` is shorthand for:
    //
    //   match expression {
    //       Some(route) => { /* use route */ }
    //       None => { /* else branch */ }
    //   }
    //
    // Use `if let` when you only care about one variant of an enum.
    // Use `match` when you need to handle all variants.
    //
    // Python equivalent:
    //   route = config.find_route(path)
    //   if route is not None:
    //       # use route
    //   else:
    //       # 404
    if let Some(route) = config.find_route(&path) {
        // Determine the path to send to the upstream server.
        // If strip_prefix is true, remove the route's prefix from the path.
        //
        // Example: path = "/api/get", prefix = "/api", strip = true
        //   stripped = "/get"
        //
        // Example: path = "/get", prefix = "/", strip = false
        //   stripped = "/get" (unchanged)
        let forwarded_path = if route.strip_prefix {
            // ===== RUST CONCEPT: .strip_prefix() (Phase 2) =====
            // Returns Option<&str>: Some(remainder) if the prefix was found,
            // None if the string doesn't start with the prefix.
            //
            // .unwrap_or() provides a fallback value if it's None.
            path.strip_prefix(&route.path_prefix).unwrap_or(&path)
        } else {
            &path
        };

        // ===== RUST CONCEPT: format! macro (Phase 1) =====
        // Like Python's f-strings: f"{upstream_base}{path}{query}"
        // Or JS template literals: `${upstream_base}${path}${query}`
        //
        // format! allocates a new String with the formatted content.
        // Build the full upstream URI.
        // If the stripped path doesn't start with /, add one.
        let upstream_uri = if forwarded_path.starts_with('/') {
            format!("{}{}{}", route.upstream, forwarded_path, query)
        } else {
            format!("{}/{}{}", route.upstream, forwarded_path, query)
        };

        // Log what we're doing (will be replaced with tracing in Phase 5)
        println!(
            "[PROXY] {} {} -> {}",
            request.method(),
            path,
            upstream_uri
        );

        // ===== RUST CONCEPT: Builder pattern (Phase 1) =====
        // Many Rust libraries use builders -- you configure an object step by step,
        // then call .build() to create the final value. This avoids constructors
        // with 15 parameters.
        //
        // Here, Client::builder() creates a partially-configured HTTP client.
        // TokioExecutor::new() tells it to use Tokio for async I/O.
        // .build_http() finalizes it as an HTTP (not HTTPS) client.
        //
        // NOTE: In a production gateway, you'd create this client ONCE and reuse it
        // across all requests (via shared state). Creating a new client per request
        // is wasteful. We'll fix this later.
        let client = Client::builder(TokioExecutor::new()).build_http();

        // ===== RUST CONCEPT: Destructuring (Phase 1) =====
        // .into_parts() splits the Request into two pieces:
        //   parts: the metadata (method, URI, headers, version)
        //   body:  the request body (POST data, file uploads, etc.)
        //
        // This CONSUMES the request (takes ownership). After this line, `request`
        // no longer exists -- its contents have been moved into `parts` and `body`.
        //
        // Python equivalent: parts, body = request.into_parts()
        // JS equivalent: const [parts, body] = request.intoParts()
        let (parts, body) = request.into_parts();

        // Build a new HTTP request targeting the upstream server.
        // We copy the original METHOD (GET, POST, etc.) and set the new URI.
        let mut upstream_req = hyper::Request::builder()
            .method(parts.method)
            .uri(&upstream_uri);

        // ===== RUST CONCEPT: Iterators (Phase 1) =====
        // .iter() creates a BORROWING iterator over the headers.
        // Each iteration yields (&HeaderName, &HeaderValue) -- references, not copies.
        //
        // Rust iterators are LAZY (like Python generators). They do nothing until
        // consumed by a for loop, .collect(), .count(), etc.
        //
        // Rust iterators are also ZERO-COST -- the compiler optimizes them into
        // the same machine code as a hand-written C loop. No overhead.
        for (key, value) in parts.headers.iter() {
            // Skip the `host` header -- we want the upstream to see its own host,
            // not "localhost:3000". This is standard reverse proxy behavior.
            if key == "host" {
                continue;
            }
            upstream_req = upstream_req.header(key, value);
        }

        // Attach the original request body and finalize the request.
        // ===== RUST CONCEPT: .unwrap() (Phase 0) =====
        // Rust has NO exceptions. Instead, operations that can fail return
        // `Result<OkValue, ErrorValue>`. You MUST handle the error case.
        //
        // .unwrap() says: "If it's Ok, give me the value. If it's Err, CRASH."
        // This is fine for quick prototyping but in production you'd handle
        // the error gracefully. We'll improve this in later phases.
        let upstream_req = upstream_req.body(body).unwrap();

        // ===== RUST CONCEPT: match expression (Phase 1) =====
        // Like Python's match/case or a switch statement, but EXHAUSTIVE.
        // You MUST handle every possible variant. The compiler rejects incomplete matches.
        //
        // Result<T, E> has two variants:
        //   Ok(value)  = success
        //   Err(error) = failure
        //
        // The `match` expression evaluates to a value (it's an expression, not a statement).
        // Both arms must return the same type -- here, both return `Response`.
        match client.request(upstream_req).await {
            Ok(response) => {
                // ===== RUST CONCEPT: .into_response() (Phase 1) =====
                // Converts the hyper Response into an Axum Response.
                // The IntoResponse trait is like a universal adapter --
                // many types know how to become an HTTP response.
                response.into_response()
            }
            Err(e) => {
                // ===== RUST CONCEPT: eprintln! (Phase 1) =====
                // Like println! but writes to stderr (standard error) instead of stdout.
                // Conventionally used for error messages and diagnostics.
                //
                // {} uses the Display trait (human-readable formatting).
                // {:?} uses the Debug trait (programmer-readable, more detailed).
                eprintln!("[PROXY ERROR] {} -> {}: {}", path, upstream_uri, e);

                // Return HTTP 502 Bad Gateway -- the standard code for "the upstream
                // server didn't respond properly".
                StatusCode::BAD_GATEWAY.into_response()
            }
        }
    } else {
        // No matching route found. This shouldn't happen if you have a "/" fallback
        // in your config, but we handle it gracefully anyway.
        eprintln!("[PROXY] No route found for path: {}", path);
        (StatusCode::NOT_FOUND, format!("No route configured for {}", path)).into_response()
    }
}
