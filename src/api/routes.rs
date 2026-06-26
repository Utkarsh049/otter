use axum::{routing::{get, post}, Extension, Router};
use axum::{middleware::{self, Next}, response::{Response, IntoResponse}, http::{Request, StatusCode}, body::Body, extract::ConnectInfo, Json};
use std::sync::Arc;
use std::net::{IpAddr, SocketAddr};
use std::time::{Instant, Duration};
use dashmap::DashMap;

use crate::config::Settings;
use crate::execution::languages::registry::LanguageRegistry;
use crate::store::memory::SubmissionStore;
use crate::queue::worker::Worker;
use super::handlers::{health, languages, submissions};

use std::sync::atomic::{AtomicU64, Ordering};

pub struct RateLimiter {
    requests: u64,
    window: Duration,
    clients: DashMap<IpAddr, (u64, Instant)>,
    access_count: AtomicU64,
}

impl RateLimiter {
    pub fn new(requests: u64, window_seconds: u64) -> Self {
        Self {
            requests,
            window: Duration::from_secs(window_seconds),
            clients: DashMap::new(),
            access_count: AtomicU64::new(0),
        }
    }

    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        
        // Periodically clean up expired entries to avoid memory leak
        let count = self.access_count.fetch_add(1, Ordering::Relaxed);
        if count % 1000 == 0 {
            let window = self.window;
            self.clients.retain(|_, (_, start_time)| {
                now.duration_since(*start_time) < window
            });
        }

        let mut entry = self.clients.entry(ip).or_insert((0, now));
        let (count, start_time) = entry.value_mut();
        
        if now.duration_since(*start_time) >= self.window {
            *count = 1;
            *start_time = now;
            true
        } else {
            if *count < self.requests {
                *count += 1;
                true
            } else {
                false
            }
        }
    }
}

pub async fn rate_limit_middleware(
    connect_info: Option<ConnectInfo<SocketAddr>>,
    limiter: Option<Extension<Arc<RateLimiter>>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    // If rate limiter is not configured, skip middleware checks
    let limiter = match limiter {
        Some(Extension(l)) => l,
        None => return next.run(req).await,
    };

    let fallback_ip = connect_info.map(|c| c.0.ip()).unwrap_or_else(|| "127.0.0.1".parse().unwrap());
    
    let ip = req.headers()
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
        .or_else(|| {
            req.headers()
                .get("x-real-ip")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.trim().parse::<IpAddr>().ok())
        })
        .unwrap_or(fallback_ip);

    if !limiter.check(ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "rate limit exceeded" }))
        ).into_response();
    }

    next.run(req).await
}

pub fn build_router(settings: Settings) -> Router {
    let registry = Arc::new(LanguageRegistry::build());
    let store    = Arc::new(SubmissionStore::new());
    let worker   = Arc::new(Worker::new(&settings, store.clone(), registry.clone()));
    build_router_with_components(settings, registry, store, worker)
}

pub fn build_router_with_components(
    settings: Settings,
    registry: Arc<LanguageRegistry>,
    store: Arc<SubmissionStore>,
    worker: Arc<Worker>,
) -> Router {
    let mut router = Router::new()
        .route("/health",             get(health::health))
        .route("/languages",          get(languages::list_languages))
        .route("/submissions",        post(submissions::submit))
        .route("/submissions/:token", get(submissions::get_submission))
        .route("/submissions/batch",   post(submissions::submit_batch))
        .route("/metrics",             get(super::handlers::metrics::get_metrics))
        .layer(Extension(registry))
        .layer(Extension(store))
        .layer(Extension(worker))
        .layer(Extension(settings.clone()));

    if let (Some(requests), Some(window_secs)) = (settings.rate_limit_requests, settings.rate_limit_window_seconds) {
        let limiter = Arc::new(RateLimiter::new(requests, window_secs));
        router = router
            .layer(middleware::from_fn(rate_limit_middleware))
            .layer(Extension(limiter));
    }

    router
}
