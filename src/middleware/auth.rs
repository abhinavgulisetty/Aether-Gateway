use axum::http::HeaderMap;

use crate::config::AuthConfig;

pub fn valid(config: &AuthConfig, headers: &HeaderMap) -> bool {
    !config.enabled
        || headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|key| config.api_keys.iter().any(|candidate| candidate == key))
}
