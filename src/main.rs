// =============================================================================
// AETHER-GATEWAY: Entry Point (main.rs)
// =============================================================================
// This is your entry point. Every Rust program starts with `fn main()`.
//
// EVOLUTION OF THIS FILE:
// - Phase 0: "Hello, Aether!" -- simplest possible Axum server
// - Phase 1: Mounted proxy module with .fallback() to forward all requests
// - Phase 2: Loaded config.yaml, injected shared state via Arc + .with_state()
// - Phase 3: Wired rate limiting + auth middleware layers (current)
//
// REQUEST FLOW (outside-in, like layers of an onion):
//   Incoming request
//     -> Rate Limiter (checks if client is within limits)
//       -> Auth Check (checks API key)
//         -> Proxy (forwards to upstream)
//       <- Auth response
//     <- Rate Limit response
//   Response to client
// =============================================================================

// ===== RUST CONCEPT: `mod` declarations (Phase 1) =====
// `mod proxy;` tells the compiler: "Look for a file called proxy.rs (or proxy/mod.rs)
// in the same directory and include it as a module named `proxy`."
//
// This is like Python's `import proxy` or JS's `import * as proxy from './proxy'`.
//
// After this declaration, you can access its public items as `proxy::forward_request`.
// The module system is hierarchical: crate::proxy::forward_request is the full path.
//
// ===== RUST CONCEPT: Module organization with directories (Phase 3) =====
// `mod middleware;` looks for either:
//   - src/middleware.rs (single file module)
//   - src/middleware/mod.rs (directory module with sub-modules)
// We use the directory form because middleware has multiple files (rate_limit.rs, auth.rs).
mod config;
mod middleware;
mod proxy;

// ===== RUST CONCEPT: `use` statements (Phase 0) =====
// Like Python's `from module import thing` or JS's `import { thing } from 'module'`.
//
// Rust's module system is EXPLICIT -- you must import every type and function
// you want to use. Nothing is available by default except a small "prelude"
// (basic types like Option, Result, Vec, String).
//
// The `::` separator is like `.` in Python/JS. So `axum::Router` means
// "the Router type from the axum crate".
//
// `middleware as axum_mw` is an alias to avoid confusion with our own `middleware` module.
use axum::{middleware as axum_mw, Router};
use config::GatewayConfig;
use std::net::SocketAddr;
use std::sync::Arc;

use middleware::rate_limit;

// ===== RUST CONCEPT: #[tokio::main] macro (Phase 0) =====
// A macro is code that generates code at compile time. The `!` in `println!`
// and the `#[...]` syntax are both macros.
//
// Rust has NO built-in async runtime. Unlike Python (which has asyncio built in)
// or Node (which has libuv built in), Rust makes you choose.
//
// #[tokio::main] does this transformation for you:
//
//   WHAT YOU WRITE:          WHAT THE COMPILER SEES:
//   #[tokio::main]           fn main() {
//   async fn main() {            tokio::runtime::Runtime::new()
//       // your code                .unwrap()
//   }                              .block_on(async { /* your code */ });
//                             }
//
// Python equivalent: asyncio.run(main())
#[tokio::main]
async fn main() {
    // ===== RUST CONCEPT: expect() vs unwrap() (Phase 2) =====
    // Both crash the program on error. The difference:
    //   .unwrap()           -> generic panic message
    //   .expect("message")  -> YOUR custom message + the error details
    //
    // Use .expect() when crashing is acceptable but you want a clear message.
    // This is appropriate here because the config file is REQUIRED -- there's
    // no reasonable fallback if it's missing or malformed.
    let config = GatewayConfig::load_from_file("config.yaml")
        .expect("Failed to load config.yaml -- make sure it exists and is valid YAML");

    // ===== RUST CONCEPT: .clone() (Phase 2) =====
    // .clone() creates a DEEP COPY. Since we're about to move `config` into
    // an Arc, we clone the listen address first so we can still use it.
    //
    // In Python, you'd just use the same variable -- everything is a reference.
    // In Rust, once you move a value, the original is gone. Clone makes a copy.
    let listen_addr = config.listen.clone();

    // Print loaded config summary
    // ===== RUST CONCEPT: println! is a MACRO, not a function (Phase 0) =====
    // The `!` means it's a macro. Macros can do things functions can't:
    // - Accept a variable number of arguments
    // - Do compile-time string formatting
    //
    // `{}` is a placeholder (like `%s` in Python or `${}` in JS template literals).
    // The difference: Rust checks format strings AT COMPILE TIME. If you
    // write `println!("{} {}", x)` with only one argument, it won't compile.
    println!("Loaded {} route(s) from config.yaml:", config.routes.len());
    // ===== RUST CONCEPT: for loop with reference (Phase 1) =====
    // `for route in &config.routes` borrows each element.
    // Without the &, we'd MOVE the routes out of config (consuming it).
    // With &, we just look at them without taking ownership.
    for route in &config.routes {
        println!(
            "  {} -> {} (strip_prefix: {})",
            route.path_prefix, route.upstream, route.strip_prefix
        );
    }
    println!(
        "Rate limiting: max_tokens={}, refill_rate={}/s",
        config.rate_limit.max_tokens, config.rate_limit.refill_rate
    );
    // ===== RUST CONCEPT: if/else as an expression (Phase 1) =====
    // In Rust, `if/else` is an EXPRESSION that returns a value.
    // This is like Python's ternary: "ENABLED" if config.auth.enabled else "disabled"
    println!(
        "Authentication: {} ({} key(s) configured)",
        if config.auth.enabled {
            "ENABLED"
        } else {
            "disabled"
        },
        config.auth.api_keys.len()
    );

    // ===== RUST CONCEPT: Arc::new() (Phase 2) =====
    // Wrap the config in an Arc so it can be shared across all request handlers.
    // Each incoming request is handled by a different Tokio task, and they all
    // need to read the config.
    //
    // Without Arc: Only one task could own the config.
    // With Arc: All tasks share a pointer to the same config data.
    //
    // Arc is read-only by default. If we needed to modify the config at runtime
    // (which we will in Phase 4b), we'd use ArcSwap or Arc<Mutex<T>>.
    let shared_config = Arc::new(config);
    let rate_limiter_state = rate_limit::new_rate_limiter_state();

    // ===== RUST CONCEPT: Method chaining / Builder pattern (Phase 0) =====
    // .route()/.fallback()/.layer() each return a NEW Router with the addition.
    // This is immutable -- each call creates a new value rather than
    // modifying the old one. Common pattern in Rust libraries.
    //
    // ===== RUST CONCEPT: Middleware layer ordering (Phase 3) =====
    // Layers are applied BOTTOM-TO-TOP in the code, but execute OUTSIDE-IN
    // for requests and INSIDE-OUT for responses.
    //
    // Think of it like wrapping a present:
    //   You wrap the innermost layer first, then add outer layers.
    //   When someone opens it, they peel the outer layer first.
    //
    // Code order (bottom to top):
    //   1. .fallback(proxy::forward_request)     <- innermost (actual work)
    //   2. .layer(auth middleware)                <- middle
    //   3. .layer(rate limit middleware)          <- outermost
    //
    // Execution order for a request:
    //   1. Rate limiter checks -> passes to auth
    //   2. Auth checks API key -> passes to proxy
    //   3. Proxy forwards to upstream
    //
    // WHY THIS ORDER?
    // Rate limiting is outermost so attackers can't brute-force your auth
    // endpoint without being throttled. Even failed auth attempts cost tokens.
    let app = Router::new()
        // ===== RUST CONCEPT: .fallback() vs .route() (Phase 1) =====
        // .route("/path", handler) matches a SPECIFIC path.
        // .fallback(handler) catches EVERY request that doesn't match a specific route.
        //
        // Since we want to proxy ALL requests (any path, any method), .fallback() is perfect.
        // In Express.js terms: app.use("*", proxyHandler)
        .fallback(proxy::forward_request)
        // ===== RUST CONCEPT: from_fn_with_state (Phase 3) =====
        // `from_fn_with_state` creates middleware from an async function,
        // injecting shared state (the rate limiter's HashMap).
        //
        // from_fn: no extra state needed
        // from_fn_with_state: needs extra state beyond what Router provides
        .layer(axum_mw::from_fn_with_state(
            shared_config.clone(),
            middleware::auth::api_key_middleware,
        ))
        .layer(axum_mw::from_fn_with_state(
            rate_limiter_state,
            rate_limit::rate_limit_middleware,
        ))
        // ===== RUST CONCEPT: .with_state() (Phase 2) =====
        // Registers shared state with the router. Any handler that takes
        // `State<Arc<GatewayConfig>>` as a parameter will receive this value.
        //
        // This is Axum's dependency injection system -- clean, type-safe, and
        // checked at compile time. If you forget .with_state(), the compiler
        // tells you instead of crashing at runtime.
        .with_state(shared_config);

    println!("\nAether-Gateway listening on {}", listen_addr);

    // ===== RUST CONCEPT: .await (Phase 0) =====
    // Like Python's `await` or JS's `await`. Suspends this function until
    // the async operation completes. Other tasks can run while we wait.
    //
    // KEY DIFFERENCE from Python/JS: In Rust, `.await` is postfix (after the
    // expression), not prefix. So it's `thing.await` not `await thing`.
    // This makes chaining nicer: `foo().await.bar().await` vs `(await (await foo()).bar())`
    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .unwrap();

    // ===== RUST CONCEPT: into_make_service_with_connect_info (Phase 3) =====
    // By default, Axum doesn't know the client's IP address.
    // `into_make_service_with_connect_info::<SocketAddr>()` tells Axum to
    // extract the connection info (IP + port) and make it available via
    // the ConnectInfo<SocketAddr> extractor in our middleware.
    //
    // Without this, ConnectInfo would not be available and the rate limiter
    // couldn't identify clients by IP.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
 