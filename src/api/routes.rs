use axum::{routing::{get, post}, Extension, Router};
use std::sync::Arc;
use crate::config::Settings;
use crate::execution::languages::registry::LanguageRegistry;
use crate::store::memory::SubmissionStore;
use super::handlers::{health, languages, submissions};

pub fn build_router(settings: Settings) -> Router {
    let registry = Arc::new(LanguageRegistry::build());
    let store    = Arc::new(SubmissionStore::new());
    Router::new()
        .route("/health",             get(health::health))
        .route("/languages",          get(languages::list_languages))
        .route("/submissions",        post(submissions::submit))
        .route("/submissions/:token", get(submissions::get_submission))
        .layer(Extension(registry))
        .layer(Extension(store))
        .layer(Extension(settings))
}
