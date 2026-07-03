// =============================================================================
// AETHER-GATEWAY: Phase 3 - Middleware Module
// =============================================================================
// This is the "mod.rs" file for the middleware module.
//
// ===== RUST CONCEPT: Module organization with directories =====
// When a module grows beyond a single file, you can split it into a directory:
//
//   middleware/
//   ├── mod.rs           <- This file. Declares sub-modules.
//   ├── rate_limit.rs    <- Rate limiting logic
//   └── auth.rs          <- Authentication logic
//
// mod.rs is like Python's __init__.py -- it's what Rust looks at when you
// write `mod middleware;` in main.rs.
//
// Each `pub mod X;` line tells Rust: "include X.rs as a public sub-module".
// External code (like main.rs) can then use `middleware::rate_limit::...`.
// =============================================================================

pub mod rate_limit;
pub mod auth;
