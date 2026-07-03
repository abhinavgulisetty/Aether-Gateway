// =============================================================================
// AETHER-GATEWAY: Phase 3 - Rate Limiting Middleware (Token Bucket)
// =============================================================================
// This module implements rate limiting using the "Token Bucket" algorithm.
//
// ANALOGY: Imagine a jar with marbles (tokens). Each request costs 1 marble.
// - The jar has a maximum capacity (e.g., 10 marbles)
// - Marbles refill at a steady rate (e.g., 2 per second)
// - If the jar is empty, the request is rejected (HTTP 429)
// - If there's a marble, take one and let the request through
//
// This allows "bursts" (use all 10 marbles at once) while maintaining
// a steady long-term rate (2 per second).
//
// HOW IT FITS IN THE REQUEST FLOW:
// User -> [Rate Limiter] -> [Auth Check] -> [Proxy to upstream]
//         ^ YOU ARE HERE
// =============================================================================

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

// ===== RUST CONCEPT: Structs as data containers =====
// This is like a Python dataclass or a simple JS class.
// No methods yet -- just data fields.
//
// #[derive(Clone)] is needed because we store these in a HashMap and
// may need to copy them. In Rust, cloning is explicit -- you choose when
// to allocate memory.
#[derive(Clone)]
pub struct TokenBucket {
    /// Current number of tokens available
    tokens: f64,
    /// Maximum tokens the bucket can hold (burst capacity)
    max_tokens: f64,
    /// How many tokens are added per second
    refill_rate: f64,
    /// When we last calculated the refill
    last_refill: Instant,
}

// ===== RUST CONCEPT: impl block with methods =====
// Methods are defined separately from the struct, in an `impl` block.
//
// `&self`     = read-only borrow (like Python's `self` but immutable)
// `&mut self` = mutable borrow (read-write access to self)
// `self`      = takes ownership (consumes the struct)
//
// Unlike Python/JS, you CHOOSE the access level for each method.
// The compiler then ensures no one else has conflicting access.
impl TokenBucket {
    /// Create a new token bucket, initially full.
    pub fn new(max_tokens: f64, refill_rate: f64) -> Self {
        // ===== RUST CONCEPT: Self =====
        // `Self` is an alias for the type being implemented (TokenBucket here).
        // It's like Python's `cls` in a classmethod.
        //
        // When field name and variable name match, you can use the shorthand:
        //   max_tokens,      is the same as     max_tokens: max_tokens,
        Self {
            tokens: max_tokens, // start with a full bucket
            max_tokens,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume one token. Returns true if allowed, false if rate limited.
    ///
    /// ===== RUST CONCEPT: &mut self =====
    /// We need `&mut self` because we're MODIFYING the bucket (reducing tokens,
    /// updating last_refill). The compiler ensures that only ONE mutable
    /// reference exists at a time -- this prevents data races.
    ///
    /// Python equivalent: def try_consume(self) -> bool:
    ///   (In Python, self is always mutable, which can cause subtle bugs in
    ///    concurrent code. Rust makes you be explicit.)
    pub fn try_consume(&mut self) -> bool {
        // Calculate how many tokens to add since last check
        let now = Instant::now();
        // ===== RUST CONCEPT: Duration arithmetic =====
        // Instant::now() is a high-precision timestamp. Subtracting two instants
        // gives a Duration. .as_secs_f64() converts to seconds as a float.
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();

        // Refill tokens, but don't exceed the maximum
        // ===== RUST CONCEPT: .min() on floats =====
        // f64.min(other) returns the smaller value. This caps the tokens.
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;

        // Try to consume one token
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true // Request allowed
        } else {
            false // Rate limited!
        }
        // Note: no explicit `return` needed -- the `if` expression IS the return value.
    }
}

// ===== RUST CONCEPT: Type alias =====
// Long generic types get hard to read. A type alias gives them a short name.
//
// This type is: Arc<Mutex<HashMap<String, TokenBucket>>>
// Let's break it down from inside out:
//
//   HashMap<String, TokenBucket>
//     - A dictionary mapping client IPs to their rate limit buckets
//     - Like Python's Dict[str, TokenBucket]
//
//   Mutex<...>
//     - A mutual exclusion lock. Only ONE task can access the HashMap at a time.
//     - This prevents data races when multiple requests arrive simultaneously.
//     - Like Python's threading.Lock(), but ENFORCED by the compiler.
//     - We use tokio::sync::Mutex (async-aware) not std::sync::Mutex.
//
//   Arc<...>
//     - Shared ownership across all request handlers (same as config).
//     - Multiple tasks can hold references to the same Mutex<HashMap>.
pub type RateLimiterState = Arc<Mutex<HashMap<String, TokenBucket>>>;

/// Create a new, empty rate limiter state.
pub fn new_rate_limiter_state() -> RateLimiterState {
    Arc::new(Mutex::new(HashMap::new()))
}

/// The actual middleware function that Axum calls for every request.
///
/// ===== RUST CONCEPT: Axum middleware with from_fn =====
/// Axum has two ways to write middleware:
///   1. `from_fn` (what we use here) -- a simple async function
///   2. Tower Service/Layer -- more powerful but much more boilerplate
///
/// The function signature matters! Axum determines what to inject by looking at types:
///   - State<T> -> extracts shared state
///   - Request -> the incoming HTTP request
///   - Next -> a handle to call the next middleware or handler
///
/// IMPORTANT: The order of parameters matters. Extractors (State, headers, etc.)
/// must come BEFORE Request and Next.
pub async fn rate_limit_middleware(
    State(limiter): State<RateLimiterState>,
    // ===== RUST CONCEPT: ConnectInfo extractor =====
    // Axum can tell us the remote socket address of the client.
    // This requires `.into_make_service_with_connect_info::<SocketAddr>()` in main.rs.
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    // Get client IP as a string for the HashMap key
    let client_ip = addr.ip().to_string();

    // ===== RUST CONCEPT: Mutex locking (.lock().await) =====
    // .lock() acquires exclusive access to the HashMap.
    // .await because this is a tokio::sync::Mutex (async-aware).
    //
    // CRITICAL: The lock is AUTOMATICALLY released when `buckets` goes
    // out of scope. This is the RAII pattern (Resource Acquisition Is
    // Initialization) -- Rust's alternative to try/finally or context managers.
    //
    // In Python:
    //   with lock:          <- acquired
    //       use_resource()
    //   # <- released here
    //
    // In Rust:
    //   let guard = lock.lock().await;  <- acquired
    //   use_resource();
    //   drop(guard);                    <- released (or when guard goes out of scope)
    let mut buckets = limiter.lock().await;

    // ===== RUST CONCEPT: HashMap::entry() API =====
    // .entry(key) is the idiomatic way to "get or insert" in a HashMap.
    //
    //   .entry(key)              -> look up the key
    //   .or_insert_with(|| ...) -> if not found, INSERT a new value using this closure
    //
    // The closure is only called if the key is missing (lazy initialization).
    // This avoids allocating a TokenBucket if the client already exists.
    //
    // Python equivalent:
    //   bucket = buckets.setdefault(client_ip, TokenBucket(10.0, 2.0))
    let bucket = buckets
        .entry(client_ip)
        .or_insert_with(|| TokenBucket::new(10.0, 2.0));

    if bucket.try_consume() {
        // ===== RUST CONCEPT: drop() -- explicit deallocation =====
        // We explicitly release the Mutex lock BEFORE calling next.run().
        // Why? Because next.run() sends the request to the upstream server,
        // which could take seconds. We don't want to HOLD the lock during
        // that time -- it would block ALL other rate limit checks.
        //
        // Without this drop(): Every request would wait for the previous
        // request's upstream call to complete before checking rate limits.
        //
        // This is a common performance pattern: acquire lock, do quick work,
        // release lock, then do slow I/O.
        drop(buckets);

        // Let the request through to the next middleware/handler
        next.run(request).await.into_response()
    } else {
        // Rate limited! Return HTTP 429 Too Many Requests.
        // The client should retry after waiting a bit.
        println!(
            "[RATE LIMIT] Blocked request from {}",
            addr.ip()
        );
        StatusCode::TOO_MANY_REQUESTS.into_response()
    }
}
