use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderName, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use std::{net::SocketAddr, sync::Arc};
use tracing::error;

use crate::{
    config::GatewayConfig,
    middleware::{auth, rate_limit::RateLimiterState},
};

pub type HttpClient = Client<HttpConnector, Body>;

pub struct ProxyState {
    pub config: GatewayConfig,
    pub client: HttpClient,
    limiter: RateLimiterState,
}

impl ProxyState {
    pub fn new(config: GatewayConfig) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        let limiter = RateLimiterState::new(config.rate_limit.clone());
        Self {
            config,
            client: Client::builder(TokioExecutor::new()).build(connector),
            limiter,
        }
    }
}

pub async fn forward_request(
    State(state): State<Arc<ProxyState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    if !state.limiter.allow(addr.ip()) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    if !auth::valid(&state.config.auth, request.headers()) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid API key").into_response();
    }

    let path = request.uri().path();
    let Some(route) = state.config.find_route(path) else {
        return (StatusCode::NOT_FOUND, "no route configured").into_response();
    };

    let forwarded_path = if route.strip_prefix {
        path.strip_prefix(&route.path_prefix).unwrap_or(path)
    } else {
        path
    };
    let forwarded_path = if forwarded_path.is_empty() {
        "/"
    } else {
        forwarded_path
    };
    let path_and_query = match request.uri().query() {
        Some(query) => format!("{forwarded_path}?{query}"),
        None => forwarded_path.to_owned(),
    };
    let upstream_uri: Uri = match format!(
        "{}{}{}",
        route.upstream.trim_end_matches('/'),
        if path_and_query.starts_with('/') {
            ""
        } else {
            "/"
        },
        path_and_query
    )
    .parse()
    {
        Ok(uri) => uri,
        Err(error) => {
            error!(%error, "invalid upstream URI");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let (mut parts, body) = request.into_parts();
    remove_hop_by_hop_headers(&mut parts.headers);
    parts.headers.remove(header::HOST);
    parts.uri = upstream_uri.clone();
    let upstream_request = Request::from_parts(parts, body);

    match state.client.request(upstream_request).await {
        Ok(mut response) => {
            remove_hop_by_hop_headers(response.headers_mut());
            response.into_response()
        }
        Err(error) => {
            error!(%error, uri = %upstream_uri, "upstream request failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

fn remove_hop_by_hop_headers(headers: &mut HeaderMap) {
    if let Some(connection) = headers.get(header::CONNECTION).cloned()
        && let Ok(connection) = connection.to_str()
    {
        for name in connection
            .split(',')
            .filter_map(|name| HeaderName::try_from(name.trim()).ok())
        {
            headers.remove(name);
        }
    }
    for name in [
        header::CONNECTION,
        header::UPGRADE,
        HeaderName::from_static("keep-alive"),
        HeaderName::from_static("proxy-authenticate"),
        HeaderName::from_static("proxy-authorization"),
        HeaderName::from_static("te"),
        HeaderName::from_static("trailer"),
        HeaderName::from_static("transfer-encoding"),
    ] {
        headers.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_standard_and_connection_nominated_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, "keep-alive, x-private".parse().unwrap());
        headers.insert("x-private", "secret".parse().unwrap());
        headers.insert("x-end-to-end", "kept".parse().unwrap());
        remove_hop_by_hop_headers(&mut headers);
        assert!(!headers.contains_key(header::CONNECTION));
        assert!(!headers.contains_key("x-private"));
        assert!(headers.contains_key("x-end-to-end"));
    }
}
