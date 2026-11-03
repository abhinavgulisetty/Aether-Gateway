use prometheus::{
    Encoder, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::sync::Arc;

pub struct Metrics {
    pub registry: Registry,
    pub requests: IntCounterVec,
    pub active: IntGaugeVec,
    pub duration: HistogramVec,
    pub events: IntCounterVec,
    pub upstream_active: IntGaugeVec,
    pub upstream_requests: IntCounterVec,
    pub upstream_latency: HistogramVec,
    pub health: IntGaugeVec,
    pub circuit: IntGaugeVec,
    pub transitions: IntCounterVec,
    pub connections: IntCounterVec,
    pub connect_latency: Histogram,
}
pub struct RouteMetrics {
    pub metrics: Arc<Metrics>,
    pub label: String,
    pub active: IntGauge,
    pub duration: Histogram,
}
pub struct UpstreamMetrics {
    pub active: IntGauge,
    pub requests: IntCounter,
    pub latency: Histogram,
    pub health: IntGauge,
    pub circuit: IntGauge,
    pub transitions: IntCounter,
}
impl Metrics {
    pub fn new() -> Arc<Self> {
        let registry = Registry::new();
        macro_rules! register {
            ($metric:expr) => {{
                let metric = $metric.unwrap();
                registry.register(Box::new(metric.clone())).unwrap();
                metric
            }};
        }
        let buckets = vec![
            0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ];
        let histogram =
            |name: &str, help: &str| HistogramOpts::new(name, help).buckets(buckets.clone());
        let result = Self {
            requests: register!(IntCounterVec::new(
                Opts::new(
                    "aether_requests_total",
                    "Responses by route and header status; stream errors counted separately"
                ),
                &["route", "status"]
            )),
            active: register!(IntGaugeVec::new(
                Opts::new(
                    "aether_active_requests",
                    "Requests including response body lifetime"
                ),
                &["route"]
            )),
            duration: register!(HistogramVec::new(
                histogram(
                    "aether_request_duration_seconds",
                    "Full response body lifetime including downstream backpressure"
                ),
                &["route"]
            )),
            events: register!(IntCounterVec::new(
                Opts::new(
                    "aether_events_total",
                    "Bounded event names: auth, rate_limit, overload, retry, timeout, upstream_error, stream_error, cancelled, no_upstream, body_limit"
                ),
                &["route", "event"]
            )),
            upstream_active: register!(IntGaugeVec::new(
                Opts::new(
                    "aether_upstream_active_requests",
                    "Selected attempts through body completion"
                ),
                &["route", "upstream"]
            )),
            upstream_requests: register!(IntCounterVec::new(
                Opts::new(
                    "aether_upstream_requests_total",
                    "Forwarding attempts excluding health probes"
                ),
                &["route", "upstream"]
            )),
            upstream_latency: register!(HistogramVec::new(
                histogram(
                    "aether_upstream_header_seconds",
                    "Attempt time to response headers or failure"
                ),
                &["route", "upstream"]
            )),
            health: register!(IntGaugeVec::new(
                Opts::new(
                    "aether_upstream_healthy",
                    "Last active probe: 1 healthy, 0 unhealthy/startup"
                ),
                &["route", "upstream"]
            )),
            circuit: register!(IntGaugeVec::new(
                Opts::new("aether_circuit_state", "0 closed, 1 open, 2 half-open"),
                &["route", "upstream"]
            )),
            transitions: register!(IntCounterVec::new(
                Opts::new("aether_circuit_transitions_total", "Circuit state changes"),
                &["route", "upstream"]
            )),
            connections: register!(IntCounterVec::new(
                Opts::new(
                    "aether_connector_total",
                    "Physical connection/TLS attempts; result started/success/error/timeout; includes probes"
                ),
                &["result"]
            )),
            connect_latency: register!(Histogram::with_opts(histogram(
                "aether_connect_duration_seconds",
                "Physical connection including DNS and TLS"
            ))),
            registry: registry.clone(),
        };
        #[cfg(target_os = "linux")]
        registry
            .register(Box::new(
                prometheus::process_collector::ProcessCollector::for_self(),
            ))
            .unwrap();
        Arc::new(result)
    }
    pub fn route(self: &Arc<Self>, label: &str) -> Arc<RouteMetrics> {
        for event in [
            "auth",
            "rate_limit",
            "overload",
            "retry",
            "timeout",
            "upstream_error",
            "stream_error",
            "cancelled",
            "no_upstream",
            "body_limit",
        ] {
            self.events.with_label_values(&[label, event]);
        }
        for status in [
            "200", "401", "404", "413", "429", "499", "500", "501", "502", "503", "504",
        ] {
            self.requests.with_label_values(&[label, status]);
        }
        Arc::new(RouteMetrics {
            metrics: self.clone(),
            label: label.into(),
            active: self.active.with_label_values(&[label]),
            duration: self.duration.with_label_values(&[label]),
        })
    }
    pub fn upstream(&self, route: &str, upstream: &str) -> UpstreamMetrics {
        let labels = &[route, upstream];
        UpstreamMetrics {
            active: self.upstream_active.with_label_values(labels),
            requests: self.upstream_requests.with_label_values(labels),
            latency: self.upstream_latency.with_label_values(labels),
            health: self.health.with_label_values(labels),
            circuit: self.circuit.with_label_values(labels),
            transitions: self.transitions.with_label_values(labels),
        }
    }
    pub fn render(&self) -> String {
        let mut buffer = Vec::new();
        TextEncoder::new()
            .encode(&self.registry.gather(), &mut buffer)
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }
}
impl RouteMetrics {
    pub fn event(&self, event: &str) {
        self.metrics
            .events
            .with_label_values(&[self.label.as_str(), event])
            .inc();
    }
    pub fn status(&self, status: u16) {
        self.metrics
            .requests
            .with_label_values(&[&self.label, &status.to_string()])
            .inc();
    }
}
