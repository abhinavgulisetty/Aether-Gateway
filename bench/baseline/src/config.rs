use serde::Deserialize;
use std::{error::Error, fs};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    pub listen: String,
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    pub path_prefix: String,
    pub upstream: String,
    #[serde(default)]
    pub strip_prefix: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: f64,
    #[serde(default = "default_refill_rate")]
    pub refill_rate: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_keys: Vec<String>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_tokens: default_max_tokens(),
            refill_rate: default_refill_rate(),
        }
    }
}

fn yes() -> bool {
    true
}
fn default_max_tokens() -> f64 {
    10.0
}
fn default_refill_rate() -> f64 {
    2.0
}

impl GatewayConfig {
    pub fn load_from_file(path: &str) -> Result<Self, Box<dyn Error>> {
        let config: Self = serde_yaml::from_str(&fs::read_to_string(path)?)?;
        config.validate()?;
        Ok(config)
    }

    pub fn find_route(&self, path: &str) -> Option<&RouteConfig> {
        self.routes.iter().find(|route| {
            path == route.path_prefix
                || (path.starts_with(&route.path_prefix)
                    && (route.path_prefix.ends_with('/')
                        || path.as_bytes().get(route.path_prefix.len()) == Some(&b'/')))
        })
    }

    fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self.routes.is_empty() {
            return Err("at least one route is required".into());
        }
        if self.rate_limit.enabled
            && (self.rate_limit.max_tokens < 1.0 || self.rate_limit.refill_rate <= 0.0)
        {
            return Err("rate_limit values must be positive".into());
        }
        if self.auth.enabled && self.auth.api_keys.is_empty() {
            return Err("auth is enabled but api_keys is empty".into());
        }
        for route in &self.routes {
            if !route.path_prefix.starts_with('/') {
                return Err(
                    format!("route prefix must start with '/': {}", route.path_prefix).into(),
                );
            }
            let uri: hyper::Uri = route.upstream.parse()?;
            if uri.scheme_str().is_none() || uri.authority().is_none() {
                return Err(format!("upstream must be an absolute URI: {}", route.upstream).into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GatewayConfig {
        serde_yaml::from_str(
            "listen: 127.0.0.1:3000\nroutes:\n  - path_prefix: /api\n    upstream: http://localhost:8080\n  - path_prefix: /\n    upstream: http://localhost:8080\n",
        ).unwrap()
    }

    #[test]
    fn route_prefix_matches_a_path_segment() {
        let config = config();
        assert_eq!(config.find_route("/api/users").unwrap().path_prefix, "/api");
        assert_eq!(config.find_route("/apiculture").unwrap().path_prefix, "/");
    }

    #[test]
    fn validation_rejects_enabled_auth_without_keys() {
        let mut config = config();
        config.auth.enabled = true;
        assert!(config.validate().is_err());
    }
}
