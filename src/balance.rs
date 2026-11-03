use crate::{
    config::{Policy, Resilience, RouteConfig},
    metrics::{Metrics, RouteMetrics, UpstreamMetrics},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tracing::info;

#[derive(Clone, Copy)]
enum Circuit {
    Closed,
    Open(Instant),
    HalfOpen,
}
struct Breaker {
    state: Circuit,
    failures: u32,
    generation: u64,
}
pub struct Upstream {
    pub origin: String,
    pub healthy: AtomicBool,
    active: AtomicUsize,
    breaker: Mutex<Breaker>,
    resilience: Resilience,
    pub metrics: UpstreamMetrics,
}
pub struct Route {
    pub config: RouteConfig,
    pub upstreams: Vec<Arc<Upstream>>,
    pub metrics: Arc<RouteMetrics>,
    cursor: Mutex<usize>,
}
pub struct Lease {
    pub upstream: Arc<Upstream>,
    generation: u64,
    finished: bool,
}
impl Route {
    pub fn new(config: RouteConfig, resilience: &Resilience, metrics: &Arc<Metrics>) -> Self {
        let upstreams = config
            .origins()
            .iter()
            .map(|origin| {
                Arc::new(Upstream {
                    origin: origin.trim_end_matches('/').to_string(),
                    healthy: AtomicBool::new(false),
                    active: AtomicUsize::new(0),
                    breaker: Mutex::new(Breaker {
                        state: Circuit::Closed,
                        failures: 0,
                        generation: 0,
                    }),
                    resilience: resilience.clone(),
                    metrics: metrics.upstream(&config.path_prefix, origin.trim_end_matches('/')),
                })
            })
            .collect();
        Self {
            metrics: metrics.route(&config.path_prefix),
            config,
            upstreams,
            cursor: Mutex::new(0),
        }
    }
    pub fn select(&self, avoid: Option<usize>) -> Option<(usize, Lease)> {
        let n = self.upstreams.len();
        if n == 1 {
            return self.upstreams[0].reserve().map(|lease| (0, lease));
        }
        // Selection + reservation is atomic across callers. No lock crosses an
        // await. The rotating scan needs no per-request allocation or sort.
        let mut cursor = self.cursor.lock().unwrap();
        if matches!(self.config.policy, Policy::RoundRobin) {
            for offset in 0..n {
                let i = (*cursor + offset) % n;
                if Some(i) == avoid {
                    continue;
                }
                if let Some(lease) = self.upstreams[i].reserve() {
                    *cursor = (i + 1) % n;
                    return Some((i, lease));
                }
            }
            if let Some(i) = avoid
                && let Some(lease) = self.upstreams[i].reserve()
            {
                *cursor = (i + 1) % n;
                return Some((i, lease));
            }
        } else {
            // Configuration caps each route at 32 peers, so this mask is fixed
            // storage. Re-scan if a circuit changes between inspection and claim.
            let mut tried = 0u32;
            for _ in 0..n {
                let candidate = (0..n)
                    .map(|offset| (*cursor + offset) % n)
                    .filter(|&i| tried & (1u32 << i) == 0 && self.upstreams[i].available())
                    .min_by_key(|&i| {
                        (
                            Some(i) == avoid,
                            self.upstreams[i].active.load(Ordering::Relaxed),
                        )
                    });
                let Some(i) = candidate else {
                    break;
                };
                tried |= 1u32 << i;
                if let Some(lease) = self.upstreams[i].reserve() {
                    *cursor = (i + 1) % n;
                    return Some((i, lease));
                }
            }
        }
        None
    }
}
impl Upstream {
    pub fn available(&self) -> bool {
        if !self.healthy.load(Ordering::Relaxed) {
            return false;
        }
        match self.breaker.lock().unwrap().state {
            Circuit::Closed => true,
            Circuit::Open(until) => Instant::now() >= until,
            Circuit::HalfOpen => false,
        }
    }
    fn transition(&self, b: &mut Breaker, state: Circuit, value: i64) {
        b.state = state;
        b.generation += 1;
        self.metrics.circuit.set(value);
        self.metrics.transitions.inc();
        info!(upstream = %self.origin, state = value, "circuit transition");
    }
    fn reserve(self: &Arc<Self>) -> Option<Lease> {
        if !self.healthy.load(Ordering::Relaxed) {
            return None;
        }
        let mut b = self.breaker.lock().unwrap();
        match b.state {
            Circuit::HalfOpen => return None,
            Circuit::Open(until) if Instant::now() < until => return None,
            Circuit::Open(_) => self.transition(&mut b, Circuit::HalfOpen, 2),
            Circuit::Closed => {}
        }
        self.active.fetch_add(1, Ordering::Relaxed);
        self.metrics.active.inc();
        self.metrics.requests.inc();
        Some(Lease {
            upstream: self.clone(),
            generation: b.generation,
            finished: false,
        })
    }
    fn complete(&self, generation: u64, success: bool) {
        let mut b = self.breaker.lock().unwrap();
        // A late completion from before an open/half-open transition cannot close
        // a newer circuit or consume its only trial request.
        if b.generation != generation {
            return;
        }
        if success {
            b.failures = 0;
            if matches!(b.state, Circuit::HalfOpen) {
                self.transition(&mut b, Circuit::Closed, 0);
            }
        } else {
            b.failures = b.failures.saturating_add(1);
            if b.failures >= self.resilience.failure_threshold
                || matches!(b.state, Circuit::HalfOpen)
            {
                self.transition(
                    &mut b,
                    Circuit::Open(
                        Instant::now() + Duration::from_millis(self.resilience.cooldown_ms),
                    ),
                    1,
                );
            }
        }
    }
}
impl Lease {
    pub fn finish(&mut self, success: bool) {
        if !self.finished {
            self.upstream.complete(self.generation, success);
            self.finished = true;
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Cancellation of the single half-open trial must not wedge the circuit.
        if !self.finished {
            let mut b = self.upstream.breaker.lock().unwrap();
            if b.generation == self.generation && matches!(b.state, Circuit::HalfOpen) {
                self.upstream.transition(
                    &mut b,
                    Circuit::Open(
                        Instant::now()
                            + Duration::from_millis(self.upstream.resilience.cooldown_ms),
                    ),
                    1,
                );
            }
        }
        self.upstream.active.fetch_sub(1, Ordering::Relaxed);
        self.upstream.metrics.active.dec();
    }
}
