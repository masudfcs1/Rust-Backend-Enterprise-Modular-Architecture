use crate::shared::AppError;
use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct AuthRateLimiter {
    buckets: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    limit: u32,
}
struct Bucket {
    started: Instant,
    count: u32,
}
const WINDOW: Duration = Duration::from_secs(60);
const MAX_CLIENTS: usize = 10_000;

impl AuthRateLimiter {
    pub fn new(limit: u32) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            limit,
        }
    }
    fn allow(&self, ip: IpAddr, now: Instant) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        // Cleanup is bounded and needed only when adding a new client at capacity.
        if !buckets.contains_key(&ip) && buckets.len() >= MAX_CLIENTS {
            buckets.retain(|_, b| now.duration_since(b.started) < WINDOW);
            if buckets.len() >= MAX_CLIENTS {
                return false;
            }
        }
        let bucket = buckets.entry(ip).or_insert(Bucket {
            started: now,
            count: 0,
        });
        if now.duration_since(bucket.started) >= WINDOW {
            bucket.started = now;
            bucket.count = 0;
        }
        if bucket.count >= self.limit {
            return false;
        }
        bucket.count += 1;
        true
    }
}

pub async fn enforce(
    State(limiter): State<AuthRateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    // Trust the socket peer only. Forwarded headers are attacker-controlled unless a proxy validates them.
    let Some(ConnectInfo(peer)) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied()
    else {
        return AppError::Unavailable.into_response();
    };
    if !limiter.allow(peer.ip(), Instant::now()) {
        return AppError::TooManyRequests.into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_each_client_and_resets_after_window() {
        let limiter = AuthRateLimiter::new(2);
        let now = Instant::now();
        let ip = "127.0.0.1".parse().unwrap();
        assert!(limiter.allow(ip, now));
        assert!(limiter.allow(ip, now));
        assert!(!limiter.allow(ip, now));
        assert!(limiter.allow("127.0.0.2".parse().unwrap(), now));
        assert!(limiter.allow(ip, now + WINDOW));
    }
}
