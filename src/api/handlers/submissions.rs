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
use crate::queue::worker::Worker;
use crate::config::Settings;
use crate::execution::limits::Limits;

pub async fn submit(
    Extension(settings): Extension<Settings>,
    Extension(registry): Extension<Arc<LanguageRegistry>>,
    Extension(store): Extension<Arc<SubmissionStore>>,
    Extension(worker): Extension<Arc<Worker>>,
    Json(req): Json<SubmissionRequest>,
) -> Result<Json<SubmissionResponse>, ApiError> {
    if registry.get(&req.language).is_none() {
        return Err(ApiError::BadRequest(
            format!("unsupported language: '{}'", req.language)
        ));
    }
    
    let token = Uuid::new_v4().to_string();
    store.insert(token.clone(), StatusCode::queued());
    
    let limits = Limits {
        cpu_time_ms: req.cpu_time_limit_ms.unwrap_or(settings.cpu_limit_ms),
        wall_time_ms: req.wall_time_limit_ms.unwrap_or(settings.wall_limit_ms),
        memory_mb: req.memory_limit_mb.unwrap_or(settings.memory_limit_mb),
        max_output_bytes: settings.max_output_bytes,
        max_processes: 32,
    };
    
    worker.enqueue(
        token.clone(),
        req.language,
        req.source_code,
        req.stdin,
        limits,
    );
    
    Ok(Json(SubmissionResponse {
        token,
        status: StatusCode::queued(),
        stdout: None,
        stderr: None,
        compile_output: None,
        time_ms: None,
        memory_kb: None,
        exit_code: None,
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