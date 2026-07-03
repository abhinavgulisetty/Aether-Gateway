// =============================================================================
// AETHER-GATEWAY: Phase 3 - Authentication Middleware
// =============================================================================
// This middleware checks for a valid API key in the `x-api-key` header.
// If the key is missing or invalid, the request is rejected with HTTP 401.
//
// HOW IT FITS IN THE REQUEST FLOW:
// User -> [Rate Limiter] -> [Auth Check] -> [Proxy to upstream]
//                            ^ YOU ARE HERE
//
// IMPORTANT: This runs AFTER rate limiting. So even if someone sends
// invalid credentials, they still consume a rate limit token. This prevents
// brute-force attacks from bypassing the rate limiter.
// =============================================================================

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use std::sync::Arc;

use crate::config::GatewayConfig;

/// Middleware that checks the `x-api-key` header against configured API keys.
///
/// ===== RUST CONCEPT: Middleware function signature =====
/// This function is called for EVERY request (before the proxy handler).
/// It decides: let the request through (call next.run()) or reject it.
///
/// The pattern is:
///   1. Extract what you need (headers, state, etc.)
///   2. Make a decision (check auth, validate input, etc.)
///   3. Either call `next.run(request)` to continue, or return an error response
///
/// This is like Express.js middleware:
///   function authMiddleware(req, res, next) {
///       if (req.headers['x-api-key'] !== 'secret') {
///           return res.status(401).send('Unauthorized');
///       }
///       next();
///   }
pub async fn api_key_middleware(
    State(config): State<Arc<GatewayConfig>>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    // If auth is disabled in config, let everything through
    if !config.auth.enabled {
        return next.run(request).await.into_response();
    }

    // ===== RUST CONCEPT: Method chaining with Option =====
    // This chain safely navigates through potentially-missing values.
    // Each method transforms the Option, and if ANY step is None,
    // the whole chain short-circuits to None.
    //
    // Step by step:
    //   .get("x-api-key")      -> Option<&HeaderValue>   (header might not exist)
    //   .and_then(|v| ...)      -> Option<&str>           (value might not be valid UTF-8)
    //   .is_some_and(|key| ...) -> bool                   (check if key is in our list)
    //
    // Python equivalent:
    //   key = request.headers.get('x-api-key')
    //   is_valid = key is not None and key in config.auth.api_keys
    //
    // The Rust version is MORE explicit about what can fail and handles
    // each failure case. The compiler ensures you don't forget any.
    let is_valid = request
        .headers()
        .get("x-api-key")
        // ===== RUST CONCEPT: .and_then() =====
        // Like .map() but for when the transform itself returns Option.
        // .map(f): Some(x) -> Some(f(x))
        // .and_then(f): Some(x) -> f(x)  (f returns Option, avoids Option<Option<T>>)
        //
        // .to_str() can fail because HTTP header values can contain non-UTF-8 bytes.
        // .ok() converts Result -> Option (Err becomes None).
        .and_then(|v| v.to_str().ok())
        // ===== RUST CONCEPT: .is_some_and() =====
        // Returns true ONLY if the Option is Some AND the predicate is true.
        // New in Rust 1.70, it's a cleaner version of .map_or(false, |x| predicate(x))
        .is_some_and(|key| {
            // ===== RUST CONCEPT: Iterator .any() =====
            // .iter().any(|item| condition) is like Python's:
            //   any(condition for item in items)
            //
            // Check if the provided key matches ANY configured key.
            config.auth.api_keys.iter().any(|valid_key| valid_key == key)
        });

    if is_valid {
        // Valid API key -- let the request through
        next.run(request).await.into_response()
    } else {
        // Invalid or missing API key -- reject with 401
        println!(
            "[AUTH] Rejected request to {} -- invalid or missing API key",
            request.uri().path()
        );
        // ===== RUST CONCEPT: Tuple as response =====
        // Axum lets you return a tuple of (StatusCode, Body) as a response.
        // The first element sets the status, the second becomes the body.
        // This is one of many types that implement IntoResponse.
        (
            StatusCode::UNAUTHORIZED,
            "Unauthorized: missing or invalid x-api-key header",
        )
            .into_response()
    }
}
