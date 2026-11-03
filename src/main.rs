mod balance;
mod body;
mod client;
mod config;
mod health;
mod metrics;
mod middleware;
mod proxy;

use axum::{
    Router,
    extract::State,
    http::{StatusCode, header},
    routing::get,
    serve::ListenerExt,
};
use config::GatewayConfig;
use proxy::ProxyState;
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tracing::{info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "aether_gateway=info".into()),
        )
        .init();
    let config_path = env::args().nth(1).unwrap_or_else(|| "config.yaml".into());
    let config = GatewayConfig::load_from_file(&config_path)
        .unwrap_or_else(|error| panic!("failed to load {config_path}: {error}"));
    let listen = config.listen.clone();
    let shutdown_seconds = config.limits.shutdown_seconds;
    let state = Arc::new(ProxyState::new(config));
    let mut probes = health::spawn(&state);
    let app = Router::new()
        .route("/health", get(|| async { "ok\n" }))
        .route(
            "/ready",
            get(|State(state): State<Arc<ProxyState>>| async move {
                if state
                    .routes
                    .iter()
                    .all(|r| r.upstreams.iter().any(|u| u.available()))
                {
                    (StatusCode::OK, "ready\n")
                } else {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "a route has no healthy upstream\n",
                    )
                }
            }),
        )
        .route(
            "/metrics",
            get(|State(state): State<Arc<ProxyState>>| async move {
                (
                    [(
                        header::CONTENT_TYPE,
                        "text/plain; version=0.0.4; charset=utf-8",
                    )],
                    state.metrics.render(),
                )
            }),
        )
        .fallback(proxy::forward_request)
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .unwrap()
        .tap_io(|stream| {
            if let Err(error) = stream.set_nodelay(true) {
                warn!(%error, "TCP_NODELAY failed");
            }
        });
    info!(%listen, "Aether Gateway listening");
    let (draining, mut drain_started) = tokio::sync::oneshot::channel();
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        let _ = draining.send(());
    });
    let server = std::future::IntoFuture::into_future(server);
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => { result.unwrap(); },
        _ = &mut drain_started => {
            info!(shutdown_seconds, "draining in-flight requests");
            if tokio::time::timeout(Duration::from_secs(shutdown_seconds), &mut server).await.is_err() {
                warn!("shutdown drain deadline exceeded; closing remaining connections");
            }
        }
    }
    probes.abort_all();
}
async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.unwrap() };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .unwrap()
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}
