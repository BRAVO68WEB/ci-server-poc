//! Authentication and authorization middleware

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AuthState {
    pub api_keys: Arc<HashSet<String>>,
    pub rate_limiter: Arc<RateLimiter>,
}

pub struct RateLimiter {
    requests: Arc<RwLock<lru::LruCache<String, Vec<Instant>>>>,
    limit: u32,
    window: Duration,
}

impl RateLimiter {
    pub fn new(limit_per_minute: u32) -> Self {
        Self {
            requests: Arc::new(RwLock::new(lru::LruCache::unbounded())),
            limit: limit_per_minute,
            window: Duration::from_secs(60),
        }
    }

    pub async fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut cache = self.requests.write().await;
        
        // Clean old entries
        if let Some(requests) = cache.get_mut(key) {
            requests.retain(|&time| now.duration_since(time) < self.window);
            
            if requests.len() >= self.limit as usize {
                return false;
            }
            
            requests.push(now);
        } else {
            cache.put(key.to_string(), vec![now]);
        }
        
        true
    }
}

// Auth middleware removed - using route-level auth checks instead
// This simplifies the implementation and avoids EitherBody complexity

