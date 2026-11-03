use crate::{
    balance::Route,
    body::{DeadlineBody, Observation},
    client::{self, HttpClient},
    config::GatewayConfig,
    metrics::{Metrics, RouteMetrics},
    middleware::{auth, rate_limit::RateLimiterState},
};
use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderName, StatusCode, Uri, Version, header},
    response::{IntoResponse, Response},
};
use http_body::Body as _;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{sync::Semaphore, time::Instant};
use tracing::warn;

pub struct ProxyState {
    pub config: GatewayConfig,
    pub client: HttpClient,
    pub routes: Vec<Route>,
    pub metrics: Arc<Metrics>,
    unmatched: Arc<RouteMetrics>,
    limiter: RateLimiterState,
    admission: Arc<Semaphore>,
}
impl ProxyState {
    pub fn new(config: GatewayConfig) -> Self {
        let metrics = Metrics::new();
        let routes = config
            .routes
            .iter()
            .map(|route| Route::new(route.clone(), &config.resilience, &metrics))
            .collect();
        Self {
            client: client::build(&config, metrics.clone()),
            routes,
            unmatched: metrics.route("unmatched"),
            metrics,
            limiter: RateLimiterState::new(config.rate_limit.clone()),
            admission: Arc::new(Semaphore::new(config.limits.max_in_flight)),
            config,
        }
    }
}
fn error(code: StatusCode, message: &'static str, mut observation: Observation) -> Response {
    observation.status(code.as_u16());
    observation.finish(true);
    let mut response = (code, message).into_response();
    if matches!(
        code,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
    ) {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, "1".parse().unwrap());
    }
    response
}

pub async fn forward_request(
    State(state): State<Arc<ProxyState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let route = state
        .routes
        .iter()
        .find(|r| r.config.matches(request.uri().path()));
    let mut observation =
        Observation::new(route.map_or_else(|| state.unmatched.clone(), |r| r.metrics.clone()));
    let metrics = observation.metrics.clone();
    if !state.limiter.allow(addr.ip()) {
        metrics.event("rate_limit");
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded\n",
            observation,
        );
    }
    if !auth::valid(&state.config.auth, request.headers()) {
        metrics.event("auth");
        return error(
            StatusCode::UNAUTHORIZED,
            "missing or invalid API key\n",
            observation,
        );
    }
    let Some(route) = route else {
        return error(StatusCode::NOT_FOUND, "no route configured\n", observation);
    };
    let Ok(permit) = state.admission.clone().try_acquire_owned() else {
        metrics.event("overload");
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway at capacity\n",
            observation,
        );
    };
    observation.permit = Some(permit);
    if request.body().size_hint().lower() > state.config.limits.max_body_bytes as u64 {
        metrics.event("body_limit");
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body too large\n",
            observation,
        );
    }
    if request.method() == axum::http::Method::CONNECT
        || request.headers().contains_key(header::UPGRADE)
    {
        return error(
            StatusCode::NOT_IMPLEMENTED,
            "tunnels and protocol upgrades are unsupported\n",
            observation,
        );
    }
    let path = request.uri().path();
    let path = if route.config.strip_prefix {
        path.strip_prefix(&route.config.path_prefix).unwrap_or(path)
    } else {
        path
    };
    let path = if path.is_empty() { "/" } else { path };
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let path_and_query = match request.uri().query() {
        Some(q) => format!("{path}?{q}"),
        None => path,
    };
    let replayable = request.body().is_end_stream()
        && matches!(request.method().as_str(), "GET" | "HEAD" | "OPTIONS");
    let (mut parts, body) = request.into_parts();
    remove_hop_by_hop_headers(&mut parts.headers);
    parts.headers.remove(header::HOST);
    // Request::new below uses HTTP/1.1; ALPN can still negotiate upstream h2.
    // Do not propagate an inbound HTTP/2 version to plain HTTP/1 origins.
    let r = &state.config.resilience;
    let deadline = Instant::now() + Duration::from_millis(r.request_timeout_ms);
    let mut body = Some(DeadlineBody::wrap(
        body,
        deadline,
        Some(state.config.limits.max_body_bytes),
        None,
    ));
    let attempts = if replayable { r.max_attempts } else { 1 };
    let mut previous = None;
    let mut last_code = StatusCode::BAD_GATEWAY;
    for attempt in 0..attempts {
        if attempt > 0 {
            let backoff =
                Duration::from_millis(r.retry_backoff_ms.saturating_mul(1 << (attempt - 1)));
            if Instant::now() + backoff >= deadline {
                last_code = StatusCode::GATEWAY_TIMEOUT;
                break;
            }
            tokio::time::sleep(backoff).await;
        }
        let Some((index, mut lease)) = route.select(previous) else {
            metrics.event("no_upstream");
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "no healthy upstream available\n",
                observation,
            );
        };
        if attempt > 0 {
            metrics.event("retry");
        }
        previous = Some(index);
        let upstream = lease.upstream.clone();
        let uri: Uri = match format!("{}{path_and_query}", upstream.origin).parse() {
            Ok(uri) => uri,
            // A valid incoming URI can exceed http::Uri's total length bound
            // once its configured origin is prepended. Never panic on that input.
            Err(_) => {
                return error(
                    StatusCode::URI_TOO_LONG,
                    "forwarded URI too long\n",
                    observation,
                );
            }
        };
        let mut forwarded = Request::new(body.take().unwrap_or_else(Body::empty));
        *forwarded.method_mut() = parts.method.clone();
        *forwarded.version_mut() = Version::HTTP_11;
        *forwarded.uri_mut() = uri;
        *forwarded.headers_mut() = parts.headers.clone();
        let started = Instant::now();
        let attempt_deadline = deadline.min(started + Duration::from_millis(r.response_timeout_ms));
        let result =
            tokio::time::timeout_at(attempt_deadline, state.client.request(forwarded)).await;
        upstream
            .metrics
            .latency
            .observe(started.elapsed().as_secs_f64());
        match result {
            Ok(Ok(mut response)) => {
                let status = response.status();
                let failed = status.is_server_error();
                if matches!(status.as_u16(), 502..=504) && attempt + 1 < attempts {
                    lease.finish(false);
                    last_code = status;
                    // Dropping an unconsumed error body may close its HTTP/1 socket.
                    // It avoids unbounded buffering or draining before retry.
                    continue;
                }
                remove_hop_by_hop_headers(response.headers_mut());
                observation.status(status.as_u16());
                observation.upstream_success = !failed;
                observation.lease = Some(lease);
                let (parts, incoming) = response.into_parts();
                return Response::from_parts(
                    parts,
                    DeadlineBody::wrap(Body::new(incoming), deadline, None, Some(observation)),
                );
            }
            Ok(Err(error)) => {
                warn!(route = %route.config.path_prefix, upstream = %upstream.origin, attempt, %error, "upstream request failed");
                metrics.event("upstream_error");
                let timed_out = error_chain_timed_out(&error);
                if timed_out {
                    metrics.event("timeout");
                }
                lease.finish(false);
                last_code = if timed_out || Instant::now() >= deadline {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                };
            }
            Err(_) => {
                metrics.event("timeout");
                lease.finish(false);
                last_code = StatusCode::GATEWAY_TIMEOUT;
            }
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    error(
        last_code,
        "upstream unavailable or timed out\n",
        observation,
    )
}

pub fn remove_hop_by_hop_headers(headers: &mut HeaderMap) {
    // Connection can appear more than once. Collect every nominated name before
    // mutating the map; processing only the first value leaks hop-local headers.
    let names: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|name| HeaderName::try_from(name.trim()).ok())
        .collect();
    for name in names {
        headers.remove(name);
    }
    for name in [
        "connection",
        "upgrade",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
    ] {
        headers.remove(name);
    }
}

fn error_chain_timed_out(mut error: &(dyn std::error::Error + 'static)) -> bool {
    loop {
        if error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::TimedOut)
        {
            return true;
        }
        match error.source() {
            Some(source) => error = source,
            None => return false,
        }
    }
}
