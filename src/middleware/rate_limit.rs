use crate::config::RateLimitConfig;
use std::{collections::HashMap, net::IpAddr, sync::Mutex, time::Instant};

struct Bucket {
    tokens: f64,
    updated: Instant,
}
struct Buckets {
    clients: HashMap<IpAddr, Bucket>,
    next_sweep: Instant,
}
pub struct RateLimiterState {
    config: RateLimitConfig,
    buckets: Mutex<Buckets>,
}
impl RateLimiterState {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(Buckets {
                clients: HashMap::new(),
                next_sweep: Instant::now(),
            }),
        }
    }
    pub fn allow(&self, ip: IpAddr) -> bool {
        if !self.config.enabled {
            return true;
        }
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap();
        // Sweep at most once a second, never evict a busy client's depleted bucket.
        if now >= buckets.next_sweep {
            buckets
                .clients
                .retain(|_, b| now.duration_since(b.updated).as_secs() < self.config.idle_seconds);
            buckets.next_sweep = now + std::time::Duration::from_secs(1);
        }
        if !buckets.clients.contains_key(&ip) && buckets.clients.len() >= self.config.max_clients {
            return false;
        }
        let bucket = buckets.clients.entry(ip).or_insert(Bucket {
            tokens: self.config.max_tokens,
            updated: now,
        });
        bucket.tokens = (bucket.tokens
            + now.duration_since(bucket.updated).as_secs_f64() * self.config.refill_rate)
            .min(self.config.max_tokens);
        bucket.updated = now;
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }
}
