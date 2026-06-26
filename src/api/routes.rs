use axum::{routing::{get, post}, Extension, Router};
use std::sync::Arc;
use crate::config::Settings;
use crate::execution::languages::registry::LanguageRegistry;
use crate::store::memory::SubmissionStore;
use crate::queue::worker::Worker;
use super::handlers::{health, languages, submissions};

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
    Router::new()
        .route("/health",             get(health::health))
        .route("/languages",          get(languages::list_languages))
        .route("/submissions",        post(submissions::submit))
        .route("/submissions/:token", get(submissions::get_submission))
        .route("/metrics",             get(super::handlers::metrics::get_metrics))
        .layer(Extension(registry))
        .layer(Extension(store))
        .layer(Extension(worker))
        .layer(Extension(settings))
}
