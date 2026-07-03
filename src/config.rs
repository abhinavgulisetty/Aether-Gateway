// =============================================================================
// AETHER-GATEWAY: Phase 2 - Configuration Module
// =============================================================================
// This module reads config.yaml and turns it into Rust structs.
// Other modules (like proxy.rs) use these structs to make routing decisions.
//
// KEY IDEA: Configuration as Code
// Instead of hardcoding "httpbin.org" in proxy.rs, we define routes in a YAML
// file. This means you can change routing WITHOUT recompiling -- just edit
// the YAML and restart (hot reload comes in Phase 4b).
// =============================================================================

// ===== RUST CONCEPT: Derive macros =====
// #[derive(X, Y, Z)] is one of Rust's most powerful features.
// It AUTO-GENERATES implementations of traits (interfaces) for your struct.
//
// Think of it like Python's @dataclass decorator, but much more powerful:
//   @dataclass           -> #[derive(Debug, Clone)]
//   json.loads()         -> #[derive(Deserialize)]
//   json.dumps()         -> #[derive(Serialize)]
//
// The compiler literally writes the code for you at compile time.
// No runtime reflection needed (unlike Python/JS).
use serde::Deserialize;

// ===== RUST CONCEPT: Structs =====
// A struct is Rust's primary data type. Like a Python dataclass or a JS object,
// but with a FIXED shape known at compile time.
//
// Unlike Python classes, structs have NO inheritance. Rust uses COMPOSITION
// (embedding structs in other structs) and TRAITS (interfaces) instead.
//
// #[derive(Debug)]       - Lets you print with {:?} (like Python's __repr__)
// #[derive(Clone)]       - Lets you deep-copy with .clone() (since Rust has no GC)
// #[derive(Deserialize)] - Auto-generates YAML/JSON/TOML parsing code
#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    /// The address to listen on, e.g., "0.0.0.0:3000"
    pub listen: String,

    /// The list of routing rules, checked in order.
    /// ===== RUST CONCEPT: Vec<T> =====
    /// Vec is Rust's dynamic array, like Python's `list` or JS's `Array`.
    /// Vec<RouteConfig> means "a list of RouteConfig items".
    ///
    /// Key difference from Python: Vec owns its elements. When the Vec is
    /// dropped (goes out of scope), all elements are freed. No GC needed.
    pub routes: Vec<RouteConfig>,

    // ===== PHASE 3 ADDITIONS =====

    /// Rate limiting configuration. Optional in YAML thanks to #[serde(default)].
    ///
    /// ===== RUST CONCEPT: #[serde(default)] on structs =====
    /// When applied to a struct field, #[serde(default)] will use the
    /// Default trait implementation if the field is missing from the YAML.
    /// We implement Default for RateLimitConfig below to provide sensible defaults.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Authentication configuration. Optional in YAML.
    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    /// The path prefix to match, e.g., "/api"
    pub path_prefix: String,

    /// The upstream URL to forward to, e.g., "http://httpbin.org"
    pub upstream: String,

    /// Whether to remove the path_prefix before forwarding.
    /// If true: /api/get -> upstream/get
    /// If false: /api/get -> upstream/api/get
    ///
    /// ===== RUST CONCEPT: #[serde(default)] =====
    /// If this field is missing from the YAML file, use the type's default value.
    /// For `bool`, the default is `false`.
    /// This is like Python's dataclass field with `default=False`.
    #[serde(default)]
    pub strip_prefix: bool,
}

// ===== RUST CONCEPT: impl blocks =====
// Methods are defined SEPARATELY from the struct, in `impl` blocks.
// This is different from Python/JS where methods are inside the class body.
//
// You can have MULTIPLE impl blocks for the same struct (useful for organizing
// code, or when different impl blocks have different trait bounds).
//
// Think of it as:
//   Python: class GatewayConfig:
//               def load_from_file(cls, path): ...
//   Rust:   impl GatewayConfig {
//               pub fn load_from_file(path: &str) -> ... { ... }
//           }
impl GatewayConfig {
    // ===== RUST CONCEPT: Result<T, E> for error handling =====
    // Rust has NO exceptions. Functions that can fail return Result<OkType, ErrorType>.
    //
    // Result<Self, Box<dyn std::error::Error>> means:
    //   - On success: return a GatewayConfig (Self = the type we're implementing)
    //   - On failure: return ANY error type (Box<dyn Error> is like Python's Exception base)
    //
    // Box<dyn Error> explanation:
    //   Box<T>  = heap-allocate the value (needed because different errors have different sizes)
    //   dyn     = dynamic dispatch (like a Python ABC -- we don't know the exact type at compile time)
    //   Error   = the std::error::Error trait (interface) that all errors implement
    //
    // Python equivalent: def load_from_file(path: str) -> GatewayConfig:  (can raise Exception)
    pub fn load_from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // ===== RUST CONCEPT: The ? operator =====
        // The `?` at the end is Rust's error propagation operator.
        // It's syntactic sugar for:
        //
        //   match std::fs::read_to_string(path) {
        //       Ok(contents) => contents,        // unwrap the value
        //       Err(e) => return Err(e.into()),   // return error to caller
        //   }
        //
        // Python equivalent: contents = open(path).read()  (exception propagates automatically)
        //
        // The difference: In Python, exceptions propagate implicitly. In Rust, you must
        // explicitly opt in with `?`. This means you can ALWAYS see where errors might occur
        // by looking for `?` in the code. No hidden surprises.
        let contents = std::fs::read_to_string(path)?;

        // ===== RUST CONCEPT: Serde deserialization (the magic) =====
        // serde_yaml::from_str() parses the YAML string into a GatewayConfig struct.
        // The #[derive(Deserialize)] we added to our struct auto-generated the code
        // that knows how to read each field from YAML.
        //
        // Python equivalent: config = GatewayConfig(**yaml.safe_load(text))
        // JS equivalent: const config = yaml.parse(text) as GatewayConfig
        //
        // The type annotation `: GatewayConfig` tells serde WHAT struct to parse into.
        // Rust figures this out from the return type, so we could also write:
        //   let config = serde_yaml::from_str(&contents)?;
        // and it would infer the type. We're explicit here for clarity.
        let config: GatewayConfig = serde_yaml::from_str(&contents)?;

        // ===== RUST CONCEPT: Ok() wrapper =====
        // Since our return type is Result<Self, ...>, we must wrap the success
        // value in Ok(). This is explicit: the caller MUST handle both Ok and Err.
        Ok(config)
    }

    // Find the first matching route for a given request path.
    //
    // ===== RUST CONCEPT: &self (self-reference) =====
    // &self means "immutable borrow of self" -- read-only access to the struct.
    // Like Python's `def method(self):` but you can't modify self.
    //
    // &str is a borrowed string slice -- we're borrowing the path, not owning it.
    //
    // ===== RUST CONCEPT: Option<T> =====
    // Returns Some(&RouteConfig) if a matching route is found, None if not.
    // Option is Rust's replacement for null/None/undefined.
    //
    // &RouteConfig (with &) means we return a REFERENCE to the route inside
    // our Vec, not a copy. This avoids unnecessary allocations.
    //
    // Python equivalent: def find_route(self, path: str) -> Optional[RouteConfig]:
    pub fn find_route(&self, path: &str) -> Option<&RouteConfig> {
        // ===== RUST CONCEPT: Iterator methods =====
        // .iter() borrows each element of the Vec (gives &RouteConfig for each).
        // .find() returns the first element matching the predicate, wrapped in Option.
        //
        // The |route| is a CLOSURE (anonymous function). Like Python's lambda:
        //   lambda route: path.startswith(route.path_prefix)
        //
        // .find() returns Option<&&RouteConfig>... but Rust auto-derefs, so
        // we get Option<&RouteConfig> in practice.
        self.routes.iter().find(|route| path.starts_with(&route.path_prefix))
    }
}

// =============================================================================
// Phase 3: Rate Limiting and Authentication Configuration
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum number of tokens (burst capacity)
    #[serde(default = "default_max_tokens")]
    pub max_tokens: f64,

    /// Tokens added per second (sustained rate)
    #[serde(default = "default_refill_rate")]
    pub refill_rate: f64,
}

// ===== RUST CONCEPT: The Default trait =====
// Default is a standard trait that provides a "default value" for a type.
// It's like Python's `__init__` with all default parameters, or a JS class
// with default property values.
//
// #[serde(default)] on a struct field uses this implementation when the
// field is missing from the YAML. So your config.yaml doesn't need to
// include rate_limit at all -- it'll get these defaults.
//
// You can derive Default automatically (#[derive(Default)]) for structs
// where all fields have Default implementations (bool = false, f64 = 0.0,
// String = "", Vec = []). But here we want CUSTOM defaults (10.0 instead
// of 0.0), so we implement it manually.
impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_tokens: 10.0,
            refill_rate: 2.0,
        }
    }
}

// ===== RUST CONCEPT: Helper functions for serde defaults =====
// #[serde(default = "function_name")] calls a SPECIFIC function for the default.
// This is useful when you want per-field defaults different from the type's Default.
fn default_max_tokens() -> f64 {
    10.0
}
fn default_refill_rate() -> f64 {
    2.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Whether authentication is enabled at all.
    /// Default: false (so existing config.yaml files without auth still work).
    #[serde(default)]
    pub enabled: bool,

    /// List of valid API keys. Requests must include one in the `x-api-key` header.
    ///
    /// ===== RUST CONCEPT: Vec default =====
    /// Vec<String> defaults to an empty Vec (no keys configured).
    /// #[serde(default)] uses this, so the field is optional in YAML.
    #[serde(default)]
    pub api_keys: Vec<String>,
}

// We can use #[derive(Default)] here because:
//   bool defaults to false
//   Vec<String> defaults to empty vec
// Both are what we want, so no custom impl needed.
impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_keys: Vec::new(),
        }
    }
}
