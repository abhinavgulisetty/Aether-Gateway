use serde::Deserialize;
use std::{collections::HashSet, error::Error, fs};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    pub listen: String,
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub health: HealthConfig,
    #[serde(default)]
    pub resilience: Resilience,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    pub path_prefix: String,
    // The singular field keeps existing local configurations readable.
    #[serde(default)]
    pub upstream: Option<String>,
    #[serde(default)]
    pub upstreams: Vec<String>,
    #[serde(default)]
    pub strip_prefix: bool,
    #[serde(default)]
    pub policy: Policy,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    #[default]
    RoundRobin,
    LeastActive,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_in_flight: usize,
    pub max_body_bytes: usize,
    pub pool_max_idle_per_host: usize,
    pub pool_idle_seconds: u64,
    pub shutdown_seconds: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_in_flight: 1024,
            max_body_bytes: 16 * 1024 * 1024,
            pool_max_idle_per_host: 32,
            pool_idle_seconds: 30,
            shutdown_seconds: 10,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HealthConfig {
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub path: String,
}
impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            interval_ms: 1000,
            timeout_ms: 300,
            path: "/health".into(),
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Resilience {
    pub connect_timeout_ms: u64,
    pub response_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub max_attempts: usize,
    pub retry_backoff_ms: u64,
    pub failure_threshold: u32,
    pub cooldown_ms: u64,
}
impl Default for Resilience {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 300,
            response_timeout_ms: 1000,
            request_timeout_ms: 5000,
            max_attempts: 2,
            retry_backoff_ms: 20,
            failure_threshold: 3,
            cooldown_ms: 2000,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub max_tokens: f64,
    pub refill_rate: f64,
    pub max_clients: usize,
    pub idle_seconds: u64,
}
impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_tokens: 10.0,
            refill_rate: 2.0,
            max_clients: 4096,
            idle_seconds: 60,
        }
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    pub enabled: bool,
    pub api_keys: Vec<String>,
}

impl RouteConfig {
    pub fn origins(&self) -> Vec<&str> {
        self.upstream
            .iter()
            .map(String::as_str)
            .chain(self.upstreams.iter().map(String::as_str))
            .collect()
    }
    pub fn matches(&self, path: &str) -> bool {
        path == self.path_prefix
            || (path.starts_with(&self.path_prefix)
                && (self.path_prefix.ends_with('/')
                    || path.as_bytes().get(self.path_prefix.len()) == Some(&b'/')))
    }
}

impl GatewayConfig {
    pub fn load_from_file(path: &str) -> Result<Self, Box<dyn Error>> {
        let config: Self = serde_yaml::from_str(&fs::read_to_string(path)?)?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        self.listen.parse::<std::net::SocketAddr>()?;
        if self.routes.is_empty() || self.routes.len() > 64 {
            return Err("configure between 1 and 64 routes".into());
        }
        let l = &self.limits;
        let h = &self.health;
        let r = &self.resilience;
        if l.max_in_flight == 0
            || l.max_in_flight > 1_000_000
            || l.max_body_bytes == 0
            || l.pool_max_idle_per_host == 0
            || l.pool_idle_seconds == 0
            || l.shutdown_seconds == 0
            || h.interval_ms == 0
            || h.timeout_ms == 0
            || r.connect_timeout_ms == 0
            || r.response_timeout_ms == 0
            || r.request_timeout_ms == 0
            || !(1..=3).contains(&r.max_attempts)
            || r.failure_threshold == 0
            || r.cooldown_ms == 0
        {
            return Err("invalid zero/out-of-range resource or timeout setting".into());
        }
        // Bounds prevent duration overflow and accidental effectively infinite deadlines.
        if [
            h.interval_ms,
            h.timeout_ms,
            r.connect_timeout_ms,
            r.response_timeout_ms,
            r.request_timeout_ms,
            r.retry_backoff_ms,
            r.cooldown_ms,
        ]
        .iter()
        .any(|v| *v > 3_600_000)
            || [
                l.pool_idle_seconds,
                l.shutdown_seconds,
                self.rate_limit.idle_seconds,
            ]
            .iter()
            .any(|v| *v > 86400)
        {
            return Err("durations exceed local prototype limits".into());
        }
        let _: hyper::http::uri::PathAndQuery = h.path.parse()?;
        if !h.path.starts_with('/') {
            return Err("health path must start with '/'".into());
        }
        let rate = &self.rate_limit;
        if !rate.max_tokens.is_finite()
            || !rate.refill_rate.is_finite()
            || rate.max_tokens < 1.0
            || rate.refill_rate <= 0.0
            || rate.max_clients == 0
            || rate.idle_seconds == 0
        {
            return Err("rate_limit values must be finite and positive".into());
        }
        if self.auth.enabled
            && (self.auth.api_keys.is_empty() || self.auth.api_keys.iter().any(|k| k.is_empty()))
        {
            return Err("enabled auth requires nonempty keys".into());
        }
        let mut prefixes = HashSet::new();
        for route in &self.routes {
            if !route.path_prefix.starts_with('/')
                || !prefixes.insert(&route.path_prefix)
                || route.path_prefix.contains(['?', '#'])
            {
                return Err("route prefixes must be unique paths starting with '/'".into());
            }
            let origins = route.origins();
            if origins.is_empty()
                || origins.len() > 32
                || (route.upstream.is_some() && !route.upstreams.is_empty())
            {
                return Err("use upstream OR 1..32 upstreams per route".into());
            }
            let mut unique = HashSet::new();
            for origin in origins {
                let uri: hyper::Uri = origin.parse()?;
                let _: hyper::Uri =
                    format!("{}{}", origin.trim_end_matches('/'), h.path).parse()?;
                if !matches!(uri.scheme_str(), Some("http" | "https"))
                    || uri.authority().is_none()
                    || uri.authority().unwrap().as_str().contains('@')
                    || uri.path() != "/"
                    || uri.query().is_some()
                    || origin.contains('#')
                    || !unique.insert(origin.trim_end_matches('/'))
                {
                    return Err(format!(
                        "upstream must be a unique HTTP(S) origin without a path: {origin}"
                    )
                    .into());
                }
            }
        }
        Ok(())
    }
}
