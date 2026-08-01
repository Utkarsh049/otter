use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Json,
};
use axum::{
    routing::{get, post},
    Extension, Router,
};
use dashmap::DashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::handlers::{health, languages, submissions};
use crate::config::Settings;
use crate::execution::languages::registry::LanguageRegistry;
use crate::queue::worker::Worker;
use crate::store::memory::SubmissionStore;

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub struct ApiKeyExtension(pub String);

pub struct RateLimiter {
    requests: u64,
    window: Duration,
    clients: DashMap<String, (u64, Instant)>,
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

    pub fn check(&self, client_id: &str) -> bool {
        let now = Instant::now();

        // Periodically clean up expired entries to avoid memory leak
        let count = self.access_count.fetch_add(1, Ordering::Relaxed);
        if count % 1000 == 0 {
            let window = self.window;
            self.clients
                .retain(|_, (_, start_time)| now.duration_since(*start_time) < window);
        }

        let mut entry = self
            .clients
            .entry(client_id.to_string())
            .or_insert((0, now));
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

    let client_id =
        if let Some(ApiKeyExtension(api_key)) = req.extensions().get::<ApiKeyExtension>() {
            api_key.clone()
        } else {
            let fallback_ip = connect_info
                .map(|c| c.0.ip())
                .unwrap_or_else(|| "127.0.0.1".parse().unwrap());
            let ip = req
                .headers()
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
            ip.to_string()
        };

    if !limiter.check(&client_id) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "rate limit exceeded" })),
        )
            .into_response();
    }

    next.run(req).await
}

pub async fn api_key_auth_middleware(
    Extension(settings): Extension<Settings>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }

    let is_admin_route = req.uri().path().starts_with("/admin");

    // Gather expected keys for authorization
    let mut expected_keys = Vec::new();
    if is_admin_route {
        if let Some(ref admin_key) = settings.otter_admin_key {
            expected_keys.push(admin_key.as_str());
        } else if let Some(ref api_key) = settings.otter_api_key {
            expected_keys.push(api_key.as_str());
        }
    } else {
        if let Some(ref api_key) = settings.otter_api_key {
            expected_keys.push(api_key.as_str());
        }
        if let Some(ref admin_key) = settings.otter_admin_key {
            expected_keys.push(admin_key.as_str());
        }
    }

    // If no keys are configured for this route, allow anonymous access
    if expected_keys.is_empty() {
        return next.run(req).await;
    }

    // Split configured keys (supporting comma-separated list of keys)
    let mut valid_keys = Vec::new();
    for keys_str in expected_keys {
        for key in keys_str.split(',') {
            let key = key.trim();
            if !key.is_empty() {
                valid_keys.push(key);
            }
        }
    }

    // Parse Authorization header
    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    let mut authenticated_token = None;

    if let Some(auth_str) = auth_header {
        if auth_str.starts_with("Bearer ") {
            let token = &auth_str[7..];
            let token_bytes = token.as_bytes();

            use subtle::ConstantTimeEq;
            for key in valid_keys {
                let key_bytes = key.as_bytes();
                if token_bytes.len() == key_bytes.len()
                    && token_bytes.ct_eq(key_bytes).unwrap_u8() == 1
                {
                    authenticated_token = Some(token.to_string());
                    break;
                }
            }
        }
    }

    if authenticated_token.is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "unauthorized",
                "message": "Invalid or missing API key in Authorization header"
            })),
        )
            .into_response();
    }

    if let Some(token) = authenticated_token {
        req.extensions_mut().insert(ApiKeyExtension(token));
    }

    next.run(req).await
}

pub fn build_router(settings: Settings) -> Router {
    let registry = Arc::new(LanguageRegistry::build());
    let store = Arc::new(SubmissionStore::new(settings.redis_url.clone()));
    let worker = Arc::new(Worker::new(&settings, store.clone(), registry.clone()));
    build_router_with_components(settings, registry, store, worker)
}

pub fn build_router_with_components(
    settings: Settings,
    registry: Arc<LanguageRegistry>,
    store: Arc<SubmissionStore>,
    worker: Arc<Worker>,
) -> Router {
    let mut router = Router::new()
        .route("/health", get(health::health))
        .route("/languages", get(languages::list_languages))
        .route("/submissions", post(submissions::submit))
        .route("/submissions/:token", get(submissions::get_submission))
        .route("/submissions/batch", post(submissions::submit_batch))
        .route("/admin/submissions", get(submissions::list_submissions))
        .route("/admin/metrics", get(super::handlers::metrics::get_metrics));

    if let (Some(requests), Some(window_secs)) = (
        settings.rate_limit_requests,
        settings.rate_limit_window_seconds,
    ) {
        let limiter = Arc::new(RateLimiter::new(requests, window_secs));
        router = router
            .layer(middleware::from_fn(rate_limit_middleware))
            .layer(Extension(limiter));
    }

    router = router.layer(middleware::from_fn(api_key_auth_middleware));

    router = router
        .layer(Extension(registry))
        .layer(Extension(store))
        .layer(Extension(worker))
        .layer(Extension(settings.clone()));

    router
}
