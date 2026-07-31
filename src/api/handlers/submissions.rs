use axum::Extension;
use crate::api::Json;
use axum::extract::{Path, ConnectInfo};
use axum::http::HeaderMap;
use std::sync::Arc;
use std::net::{SocketAddr, IpAddr};
use uuid::Uuid;
use crate::api::errors::ApiError;
use crate::api::models::request::{SubmissionRequest, BatchSubmissionRequest};
use crate::api::models::response::{SubmissionResponse, BatchSubmissionResponse};
use crate::api::models::status::StatusCode;
use crate::execution::languages::registry::LanguageRegistry;
use crate::store::memory::SubmissionStore;
use crate::queue::worker::Worker;
use crate::config::Settings;
use crate::execution::limits::Limits;

fn get_client_ip(headers: &HeaderMap, connect_info: Option<ConnectInfo<SocketAddr>>) -> IpAddr {
    let fallback_ip = connect_info.map(|c| c.0.ip()).unwrap_or_else(|| "127.0.0.1".parse().unwrap());
    headers.get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
        .or_else(|| {
            headers.get("x-real-ip")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.trim().parse::<IpAddr>().ok())
        })
        .unwrap_or(fallback_ip)
}

pub async fn submit(
    Extension(settings): Extension<Settings>,
    Extension(registry): Extension<Arc<LanguageRegistry>>,
    Extension(store): Extension<Arc<SubmissionStore>>,
    Extension(worker): Extension<Arc<Worker>>,
    headers: HeaderMap,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    payload: Result<Json<SubmissionRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(axum::http::StatusCode, Json<SubmissionResponse>), ApiError> {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(err) => return Err(ApiError::BadRequest(err.to_string())),
    };

    req.validate(&settings)?;

    let lang = registry.get(&req.language).ok_or_else(|| {
        ApiError::BadRequest(
            format!("unsupported language: '{}'", req.language)
        )
    })?;
    
    let token = Uuid::new_v4().to_string();
    store.insert(token.clone(), StatusCode::queued()).await;
    
    let limits = Limits {
        cpu_time_ms: req.cpu_time_limit_ms.unwrap_or(settings.cpu_limit_ms),
        wall_time_ms: req.wall_time_limit_ms.unwrap_or(settings.wall_limit_ms),
        memory_mb: req.memory_limit_mb.unwrap_or(settings.memory_limit_mb),
        max_output_bytes: settings.max_output_bytes,
        max_processes: lang.default_limits().max_processes,
        disable_sandbox: settings.disable_sandbox,
        slot_id: None,
    };
    
    let ip = get_client_ip(&headers, connect_info);
    
    worker.enqueue(
        token.clone(),
        req.language,
        req.source_code,
        req.stdin,
        limits,
        ip,
        req.webhook_url,
    ).await?;
    
    Ok((
        axum::http::StatusCode::CREATED,
        Json(SubmissionResponse {
            token,
            status: StatusCode::queued(),
            stdout: None,
            stderr: None,
            compile_output: None,
            time_ms: None,
            memory_kb: None,
            exit_code: None,
        })
    ))
}

pub async fn submit_batch(
    Extension(settings): Extension<Settings>,
    Extension(registry): Extension<Arc<LanguageRegistry>>,
    Extension(store): Extension<Arc<SubmissionStore>>,
    Extension(worker): Extension<Arc<Worker>>,
    headers: HeaderMap,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    payload: Result<Json<BatchSubmissionRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(axum::http::StatusCode, Json<BatchSubmissionResponse>), ApiError> {
    let Json(req_batch) = match payload {
        Ok(json) => json,
        Err(err) => return Err(ApiError::BadRequest(err.to_string())),
    };

    // First validate all requests in the batch
    for req in &req_batch.submissions {
        req.validate(&settings)?;
        if registry.get(&req.language).is_none() {
            return Err(ApiError::BadRequest(
                format!("unsupported language: '{}'", req.language)
            ));
        }
    }

    // Check queue capacity for the entire batch
    if worker.queue_depth().await + req_batch.submissions.len() > worker.max_queue_depth() {
        return Err(ApiError::TooManyRequests(
            "server is at capacity, try again shortly".to_string()
        ));
    }

    let mut responses = Vec::new();
    let ip = get_client_ip(&headers, connect_info);

    for req in req_batch.submissions {
        let lang = registry.get(&req.language).unwrap();
        let token = Uuid::new_v4().to_string();
        store.insert(token.clone(), StatusCode::queued()).await;
        
        let limits = Limits {
            cpu_time_ms: req.cpu_time_limit_ms.unwrap_or(settings.cpu_limit_ms),
            wall_time_ms: req.wall_time_limit_ms.unwrap_or(settings.wall_limit_ms),
            memory_mb: req.memory_limit_mb.unwrap_or(settings.memory_limit_mb),
            max_output_bytes: settings.max_output_bytes,
            max_processes: lang.default_limits().max_processes,
            disable_sandbox: settings.disable_sandbox,
            slot_id: None,
        };
        
        worker.enqueue(
            token.clone(),
            req.language,
            req.source_code,
            req.stdin,
            limits,
            ip,
            req.webhook_url,
        ).await?;

        responses.push(SubmissionResponse {
            token,
            status: StatusCode::queued(),
            stdout: None,
            stderr: None,
            compile_output: None,
            time_ms: None,
            memory_kb: None,
            exit_code: None,
        });
    }

    Ok((
        axum::http::StatusCode::CREATED,
        Json(BatchSubmissionResponse { submissions: responses }),
    ))
}

pub async fn get_submission(
    Extension(store): Extension<Arc<SubmissionStore>>,
    Path(token): Path<String>,
) -> Result<Json<SubmissionResponse>, ApiError> {
    store.get(&token).await
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(
            format!("submission '{}' not found", token)
        ))
}

pub async fn list_submissions(
    Extension(store): Extension<Arc<SubmissionStore>>,
) -> Result<Json<Vec<SubmissionResponse>>, ApiError> {
    Ok(Json(store.get_all().await))
}