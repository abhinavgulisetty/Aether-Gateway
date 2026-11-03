use std::{collections::HashMap, net::IpAddr, sync::Mutex, time::Instant};

use crate::config::RateLimitConfig;

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(max_tokens: f64) -> Self {
        Self {
            tokens: max_tokens,
            last_refill: Instant::now(),
        }
    }

    fn consume(&mut self, max_tokens: f64, refill_rate: f64) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens
            + now.duration_since(self.last_refill).as_secs_f64() * refill_rate)
            .min(max_tokens);
        self.last_refill = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

struct RateLimiter {
    config: RateLimitConfig,
    // ponytail: one lock and no eviction; shard/expire after a many-IP benchmark proves need.
    buckets: HashMap<IpAddr, TokenBucket>,
}

pub struct RateLimiterState(Mutex<RateLimiter>);

impl RateLimiterState {
    pub fn new(config: RateLimitConfig) -> Self {
        Self(Mutex::new(RateLimiter {
            config,
            buckets: HashMap::new(),
        }))
    }

    pub fn allow(&self, ip: IpAddr) -> bool {
        let mut limiter = self.0.lock().unwrap();
        if !limiter.config.enabled {
            return true;
        }
        let max_tokens = limiter.config.max_tokens;
        let refill_rate = limiter.config.refill_rate;
        limiter
            .buckets
            .entry(ip)
            .or_insert_with(|| TokenBucket::new(max_tokens))
            .consume(max_tokens, refill_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_enforces_burst_capacity() {
        let mut bucket = TokenBucket::new(2.0);
        assert!(bucket.consume(2.0, 0.001));
        assert!(bucket.consume(2.0, 0.001));
        assert!(!bucket.consume(2.0, 0.001));
    }
}
