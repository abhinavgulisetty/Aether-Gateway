use axum::http::HeaderMap;

use crate::config::AuthConfig;

pub fn valid(config: &AuthConfig, headers: &HeaderMap) -> bool {
    !config.enabled
        || headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|key| config.api_keys.iter().any(|candidate| candidate == key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_auth_rejects_a_missing_key() {
        let config = AuthConfig {
            enabled: true,
            api_keys: vec!["secret".into()],
        };
        assert!(!valid(&config, &HeaderMap::new()));
    }
}
