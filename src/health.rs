use crate::proxy::ProxyState;
use axum::{body::Body, extract::Request};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::task::JoinSet;
use tracing::info;

pub fn spawn(state: &Arc<ProxyState>) -> JoinSet<()> {
    let mut tasks = JoinSet::new();
    for route in &state.routes {
        for upstream in &route.upstreams {
            let upstream = upstream.clone();
            let client = state.client.clone();
            let config = state.config.health.clone();
            tasks.spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(config.interval_ms));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let req = Request::builder()
                        .method("HEAD")
                        .uri(format!("{}{}", upstream.origin, config.path))
                        .body(Body::empty())
                        .unwrap();
                    let healthy = tokio::time::timeout(
                        Duration::from_millis(config.timeout_ms),
                        client.request(req),
                    )
                    .await
                    .is_ok_and(|response| {
                        response.is_ok_and(|response| response.status().is_success())
                    });
                    let old = upstream.healthy.swap(healthy, Ordering::Relaxed);
                    upstream.metrics.health.set(i64::from(healthy));
                    if old != healthy {
                        info!(upstream = %upstream.origin, healthy, "active health changed");
                    }
                }
            });
        }
    }
    tasks
}
