use axum::{Extension, Json};
use axum::extract::Path;
use std::sync::Arc;
use uuid::Uuid;
use crate::api::errors::ApiError;
use crate::api::models::request::SubmissionRequest;
use crate::api::models::response::SubmissionResponse;
use crate::api::models::status::StatusCode;
use crate::execution::languages::registry::LanguageRegistry;
use crate::store::memory::SubmissionStore;

pub async fn submit(
    Extension(registry): Extension<Arc<LanguageRegistry>>,
    Extension(store): Extension<Arc<SubmissionStore>>,
    Json(req): Json<SubmissionRequest>,
) -> Result<Json<SubmissionResponse>, ApiError> {
    if registry.get(&req.language).is_none() {
        return Err(ApiError::BadRequest(
            format!("unsupported language: '{}'", req.language)
        ));
    }
    let token = Uuid::new_v4().to_string();
    store.insert(token.clone(), StatusCode::queued());
    // Phase 3: enqueue for real execution
    Ok(Json(SubmissionResponse {
        token,
        status: StatusCode::queued(),
        stdout: None, stderr: None,
        compile_output: None,
        time_ms: None, memory_kb: None, exit_code: None,
    }))
}

pub async fn get_submission(
    Extension(store): Extension<Arc<SubmissionStore>>,
    Path(token): Path<String>,
) -> Result<Json<SubmissionResponse>, ApiError> {
    store.get(&token)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(
            format!("submission '{}' not found", token)
        ))
}