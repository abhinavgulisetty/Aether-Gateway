use crate::{config::GatewayConfig, metrics::Metrics};
use axum::{body::Body, http::Uri};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioTimer},
};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tower_service::Service;

type Tls = HttpsConnector<HttpConnector>;
#[derive(Clone)]
pub struct Connector {
    inner: Tls,
    metrics: Arc<Metrics>,
    timeout: Duration,
}
pub type HttpClient = Client<Connector, Body>;
type BoxError = Box<dyn std::error::Error + Send + Sync>;
impl Service<Uri> for Connector {
    type Response = <Tls as Service<Uri>>::Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }
    fn call(&mut self, uri: Uri) -> Self::Future {
        let future = self.inner.call(uri);
        let metrics = self.metrics.clone();
        let timeout = self.timeout;
        metrics.connections.with_label_values(&["started"]).inc();
        Box::pin(async move {
            let timer = metrics.connect_latency.start_timer();
            let result = tokio::time::timeout(timeout, future).await;
            timer.observe_duration();
            let outcome = match &result {
                Ok(Ok(_)) => "success",
                Ok(Err(_)) => "error",
                Err(_) => "timeout",
            };
            metrics.connections.with_label_values(&[outcome]).inc();
            result.map_err(|_| -> BoxError {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "connect deadline").into()
            })?
        })
    }
}
pub fn build(config: &GatewayConfig, metrics: Arc<Metrics>) -> HttpClient {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_nodelay(true);
    http.set_connect_timeout(Some(Duration::from_millis(
        config.resilience.connect_timeout_ms,
    )));
    let tls = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .wrap_connector(http);
    Client::builder(TokioExecutor::new())
        .pool_timer(TokioTimer::new())
        .timer(TokioTimer::new())
        .pool_idle_timeout(Duration::from_secs(config.limits.pool_idle_seconds))
        .pool_max_idle_per_host(config.limits.pool_max_idle_per_host)
        // Application retries are deliberately visible and forbidden for POST.
        .retry_canceled_requests(false)
        .build(Connector {
            inner: tls,
            metrics,
            timeout: Duration::from_millis(config.resilience.connect_timeout_ms),
        })
}
