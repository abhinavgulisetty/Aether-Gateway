mod config;
mod middleware;
mod proxy;

use axum::{Router, routing::get, serve::ListenerExt};
use config::GatewayConfig;
use proxy::ProxyState;
use std::{env, net::SocketAddr, sync::Arc};
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config_path = env::args().nth(1).unwrap_or_else(|| "config.yaml".into());
    let config = GatewayConfig::load_from_file(&config_path)
        .unwrap_or_else(|error| panic!("failed to load {config_path}: {error}"));
    let listen = config.listen.clone();
    let state = Arc::new(ProxyState::new(config));

    let gateway = Router::new()
        .fallback(proxy::forward_request)
        .with_state(state);
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(gateway)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .unwrap()
        .tap_io(|stream| stream.set_nodelay(true).unwrap());
    info!(%listen, "Aether Gateway listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap();
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
