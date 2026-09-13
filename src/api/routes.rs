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
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::handlers::{health, languages, submissions};
use super::identity::{extract_ip, verify_jwt_assertion, ClientIdentity};
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
        self.check_with_retry(client_id).0
    }

    pub fn check_with_retry(&self, client_id: &str) -> (bool, u64) {
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
            (true, 0)
        } else if *count < self.requests {
            *count += 1;
            (true, 0)
        } else {
            let elapsed = now.duration_since(*start_time);
            let remaining = self.window.saturating_sub(elapsed);
            let retry_after = remaining.as_secs().max(1);
            (false, retry_after)
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

    let client_id = if let Some(identity) = req.extensions().get::<ClientIdentity>() {
        identity.rate_limit_key()
    } else if let Some(ApiKeyExtension(api_key)) = req.extensions().get::<ApiKeyExtension>() {
        format!("key:{}", api_key)
    } else {
        let ip = extract_ip(req.headers(), connect_info);
        format!("ip:{}", ip)
    };

    let (allowed, retry_after) = limiter.check_with_retry(&client_id);
    if !allowed {
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "rate limit exceeded" })),
        )
            .into_response();
        resp.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            axum::http::HeaderValue::from(retry_after),
        );
        return resp;
    }

    next.run(req).await
}

pub async fn api_key_auth_middleware(
    connect_info: Option<ConnectInfo<SocketAddr>>,
    Extension(settings): Extension<Settings>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if req.uri().path() == "/health" {
        let ip = extract_ip(req.headers(), connect_info);
        req.extensions_mut().insert(ClientIdentity::Ip { address: ip });
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

    let mut authenticated_token = None;

    // Check Authorization header if expected keys are configured
    if !expected_keys.is_empty() {
        let mut valid_keys = Vec::new();
        for keys_str in expected_keys {
            for key in keys_str.split(',') {
                let key = key.trim();
                if !key.is_empty() {
                    valid_keys.push(key);
                }
            }
        }

        let auth_header = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok());

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
    }

    // Resolve ClientIdentity
    let assertion_header = req
        .headers()
        .get("x-otter-user-assertion")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut user_identity: Option<ClientIdentity> = None;

    if let Some(assertion) = assertion_header {
        if let Some(ref secret) = settings.otter_jwt_secret {
            match verify_jwt_assertion(
                &assertion,
                secret,
                settings.otter_jwt_issuer.as_deref(),
                settings.otter_jwt_audience.as_deref(),
            ) {
                Ok(id) => {
                    user_identity = Some(id);
                }
                Err(err) => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({
                            "error": "unauthorized",
                            "message": format!("Invalid user assertion: {}", err)
                        })),
                    )
                        .into_response();
                }
            }
        } else {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "unauthorized",
                    "message": "User assertion provided but OTTER_JWT_SECRET is not configured"
                })),
            )
                .into_response();
        }
    } else if settings.otter_identity_mode.as_deref() == Some("jwt") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "unauthorized",
                "message": "Missing required user assertion header (X-Otter-User-Assertion)"
            })),
        )
            .into_response();
    } else if settings.otter_identity_mode.as_deref() == Some("trusted_header") {
        let user_id_header = req
            .headers()
            .get("x-otter-user-id")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let tenant_id_header = req
            .headers()
            .get("x-otter-tenant-id")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(user_id) = user_id_header {
            user_identity = Some(ClientIdentity::User {
                subject: user_id,
                tenant_id: tenant_id_header,
            });
        } else {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "unauthorized",
                    "message": "Missing required identity header (X-Otter-User-Id)"
                })),
            )
                .into_response();
        }
    }

    let client_identity = if let Some(id) = user_identity {
        id
    } else if let Some(ref token) = authenticated_token {
        ClientIdentity::ApiKey {
            key_id: token.clone(),
        }
    } else {
        let ip = extract_ip(req.headers(), connect_info);
        ClientIdentity::Ip { address: ip }
    };

    req.extensions_mut().insert(client_identity);
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum_test::TestServer;

    #[tokio::test]
    async fn test_admin_route_authentication() {
        // Case 1: Both keys exist.
        // /admin/* accepts otter_admin_key, rejects otter_api_key.
        let mut settings = Settings::default();
        settings.otter_api_key = Some("client_key".to_string());
        settings.otter_admin_key = Some("admin_key".to_string());
        let app = build_router(settings);
        let server = TestServer::new(app).unwrap();

        // Check otter_admin_key is accepted on /admin/submissions
        let res = server.get("/admin/submissions")
            .add_header(
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderValue::from_static("Bearer admin_key"),
            )
            .await;
        assert_eq!(res.status_code(), axum::http::StatusCode::OK);

        // Check non-admin /languages route accepts admin_key when both exist
        let res = server.get("/languages")
            .add_header(
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderValue::from_static("Bearer admin_key"),
            )
            .await;
        assert_eq!(res.status_code(), axum::http::StatusCode::OK);

        // Check otter_api_key is rejected on /admin/submissions
        let res = server.get("/admin/submissions")
            .add_header(
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderValue::from_static("Bearer client_key"),
            )
            .await;
        assert_eq!(res.status_code(), axum::http::StatusCode::UNAUTHORIZED);

        // Case 2: Falls back to otter_api_key when no admin key exists.
        let mut settings = Settings::default();
        settings.otter_api_key = Some("client_key".to_string());
        settings.otter_admin_key = None;
        let app = build_router(settings);
        let server = TestServer::new(app).unwrap();

        let res = server.get("/admin/submissions")
            .add_header(
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderValue::from_static("Bearer client_key"),
            )
            .await;
        assert_eq!(res.status_code(), axum::http::StatusCode::OK);

        // Case 3: Permits anonymous access when neither key is configured.
        let mut settings = Settings::default();
        settings.otter_api_key = None;
        settings.otter_admin_key = None;
        let app = build_router(settings);
        let server = TestServer::new(app).unwrap();

        let res = server.get("/admin/submissions").await;
        assert_eq!(res.status_code(), axum::http::StatusCode::OK);
    }
}
