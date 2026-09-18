use crate::api::errors::ApiError;
use crate::api::identity::ClientIdentity;
use crate::api::models::request::{BatchSubmissionRequest, SubmissionRequest};
use crate::api::models::response::{BatchSubmissionResponse, SubmissionResponse};
use crate::api::models::status::StatusCode;
use crate::api::Json;
use crate::config::Settings;
use crate::execution::languages::registry::LanguageRegistry;
use crate::execution::limits::Limits;
use crate::queue::worker::Worker;
use crate::store::memory::SubmissionStore;
use axum::extract::{ConnectInfo, Path};
use axum::http::HeaderMap;
use axum::Extension;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use uuid::Uuid;

fn get_client_ip(
    headers: &HeaderMap,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    trusted_proxies: &[IpAddr],
) -> IpAddr {
    crate::api::identity::extract_ip(headers, connect_info, trusted_proxies)
}

fn sanitize_client_identity(
    identity: Option<Extension<ClientIdentity>>,
    headers: &HeaderMap,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    trusted_proxies: &[IpAddr],
) -> ClientIdentity {
    match identity {
        Some(Extension(ClientIdentity::ApiKey { key_id })) => {
            let mut hasher = sha1_smol::Sha1::new();
            hasher.update(key_id.as_bytes());
            ClientIdentity::ApiKey {
                key_id: hasher.digest().to_string(),
            }
        }
        Some(Extension(id)) => id,
        None => {
            let ip = get_client_ip(headers, connect_info, trusted_proxies);
            ClientIdentity::Ip { address: ip }
        }
    }
}

pub async fn submit(
    Extension(settings): Extension<Settings>,
    Extension(registry): Extension<Arc<LanguageRegistry>>,
    Extension(store): Extension<Arc<SubmissionStore>>,
    Extension(worker): Extension<Arc<Worker>>,
    identity: Option<Extension<ClientIdentity>>,
    headers: HeaderMap,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    payload: Result<Json<SubmissionRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(axum::http::StatusCode, Json<SubmissionResponse>), ApiError> {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(err) => return Err(ApiError::BadRequest(err.to_string())),
    };

    req.validate(&settings)?;

    let lang = registry
        .get(&req.language)
        .ok_or_else(|| ApiError::BadRequest(format!("unsupported language: '{}'", req.language)))?;

    let token = Uuid::new_v4().to_string();
    store
        .insert(token.clone(), StatusCode::queued())
        .await
        .map_err(|e| {
            tracing::error!("Failed to initialize submission: {}", e);
            ApiError::InternalError("Failed to initialize submission".to_string())
        })?;

    let limits = Limits {
        cpu_time_ms: req.cpu_time_limit_ms.unwrap_or(settings.cpu_limit_ms),
        wall_time_ms: req.wall_time_limit_ms.unwrap_or(settings.wall_limit_ms),
        memory_mb: req.memory_limit_mb.unwrap_or(settings.memory_limit_mb),
        max_output_bytes: settings.max_output_bytes,
        max_processes: lang.default_limits().max_processes,
        disable_sandbox: settings.disable_sandbox,
        slot_id: None,
    };

    let client_identity =
        sanitize_client_identity(identity, &headers, connect_info, &settings.trusted_proxies);

    if let Err(e) = worker
        .enqueue(
            token.clone(),
            req.language,
            req.source_code,
            req.stdin,
            limits,
            client_identity,
            req.webhook_url,
        )
        .await
    {
        let should_remove = matches!(
            e,
            crate::queue::worker::EnqueueError::DefinitivelyNotEnqueued(_)
        );
        if should_remove {
            if let Err(remove_err) = store.remove(&token).await {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                token.hash(&mut hasher);
                let token_hash = hasher.finish();
                tracing::error!(
                    token_hash = %token_hash,
                    error = %remove_err,
                    "Failed to clean up submission from store after enqueuing failure"
                );
            }
        }
        let api_err = match e {
            crate::queue::worker::EnqueueError::DefinitivelyNotEnqueued(err) => err,
            crate::queue::worker::EnqueueError::Indeterminate(err) => err,
        };
        let mapped_err = match api_err {
            ApiError::InternalError(m) => {
                tracing::error!("Internal error during submission enqueue: {}", m);
                ApiError::InternalError("Failed to enqueue submission".to_string())
            }
            other => other,
        };
        return Err(mapped_err);
    }

    let response = SubmissionResponse {
        token,
        status: StatusCode::queued(),
        stdout: None,
        stderr: None,
        compile_output: None,
        time_ms: None,
        memory_kb: None,
        exit_code: None,
    };

    Ok((axum::http::StatusCode::CREATED, Json(response)))
}

pub async fn get_submission(
    Path(token): Path<String>,
    Extension(store): Extension<Arc<SubmissionStore>>,
) -> Result<Json<SubmissionResponse>, ApiError> {
    let sub = store.get(&token).await.map_err(|e| {
        tracing::error!("Failed to fetch submission {}: {}", token, e);
        ApiError::InternalError("Failed to fetch submission".to_string())
    })?;

    match sub {
        Some(response) => Ok(Json(response)),
        None => Err(ApiError::NotFound(format!(
            "submission '{}' not found",
            token
        ))),
    }
}

pub async fn list_submissions(
    Extension(store): Extension<Arc<SubmissionStore>>,
) -> Result<Json<Vec<SubmissionResponse>>, ApiError> {
    let subs = store.get_all().await.map_err(|e| {
        tracing::error!("Failed to list submissions: {}", e);
        ApiError::InternalError("Failed to list submissions".to_string())
    })?;
    Ok(Json(subs))
}

pub async fn submit_batch(
    Extension(settings): Extension<Settings>,
    Extension(registry): Extension<Arc<LanguageRegistry>>,
    Extension(store): Extension<Arc<SubmissionStore>>,
    Extension(worker): Extension<Arc<Worker>>,
    identity: Option<Extension<ClientIdentity>>,
    headers: HeaderMap,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    payload: Result<Json<BatchSubmissionRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(axum::http::StatusCode, Json<BatchSubmissionResponse>), ApiError> {
    let Json(req_batch) = match payload {
        Ok(json) => json,
        Err(err) => return Err(ApiError::BadRequest(err.to_string())),
    };

    for req in &req_batch.submissions {
        req.validate(&settings)?;
        if registry.get(&req.language).is_none() {
            return Err(ApiError::BadRequest(format!(
                "unsupported language: '{}'",
                req.language
            )));
        }
    }

    let depth = worker.queue_depth().await.map_err(|e| {
        tracing::error!("Failed to query queue depth: {}", e);
        ApiError::InternalError("Failed to query queue depth".to_string())
    })?;
    if depth + req_batch.submissions.len() > worker.max_queue_depth() {
        return Err(ApiError::TooManyRequests(
            "server is at capacity, try again shortly".to_string(),
        ));
    }

    let mut responses = Vec::new();
    let client_identity =
        sanitize_client_identity(identity, &headers, connect_info, &settings.trusted_proxies);

    for req in req_batch.submissions {
        let lang = registry.get(&req.language).unwrap();
        let token = Uuid::new_v4().to_string();
        if let Err(e) = store.insert(token.clone(), StatusCode::queued()).await {
            tracing::error!("Failed to initialize submission in batch: {}", e);
            responses.push(SubmissionResponse {
                token,
                status: StatusCode {
                    id: 8,
                    description: "Failed to initialize submission".to_string(),
                },
                stdout: None,
                stderr: None,
                compile_output: None,
                time_ms: None,
                memory_kb: None,
                exit_code: None,
            });
            continue;
        }

        let limits = Limits {
            cpu_time_ms: req.cpu_time_limit_ms.unwrap_or(settings.cpu_limit_ms),
            wall_time_ms: req.wall_time_limit_ms.unwrap_or(settings.wall_limit_ms),
            memory_mb: req.memory_limit_mb.unwrap_or(settings.memory_limit_mb),
            max_output_bytes: settings.max_output_bytes,
            max_processes: lang.default_limits().max_processes,
            disable_sandbox: settings.disable_sandbox,
            slot_id: None,
        };

        match worker
            .enqueue(
                token.clone(),
                req.language,
                req.source_code,
                req.stdin,
                limits,
                client_identity.clone(),
                req.webhook_url,
            )
            .await
        {
            Ok(_) => {
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
            Err(e) => {
                let should_remove = matches!(
                    e,
                    crate::queue::worker::EnqueueError::DefinitivelyNotEnqueued(_)
                );
                if should_remove {
                    if let Err(remove_err) = store.remove(&token).await {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        token.hash(&mut hasher);
                        let token_hash = hasher.finish();
                        tracing::error!(
                            token_hash = %token_hash,
                            error = %remove_err,
                            "Failed to clean up submission from store after enqueuing failure in batch"
                        );
                    }
                }
                let api_err = match e {
                    crate::queue::worker::EnqueueError::DefinitivelyNotEnqueued(err) => err,
                    crate::queue::worker::EnqueueError::Indeterminate(err) => err,
                };
                let status_desc = match api_err {
                    ApiError::TooManyRequests(m) => format!("Rejected: {}", m),
                    ApiError::InternalError(m) => {
                        tracing::error!("Internal error during batch submission enqueue: {}", m);
                        "Internal Error: Failed to enqueue submission".to_string()
                    }
                    ApiError::BadRequest(m) => format!("Bad Request: {}", m),
                    ApiError::NotFound(m) => format!("Not Found: {}", m),
                };

                responses.push(SubmissionResponse {
                    token,
                    status: StatusCode {
                        id: 8,
                        description: status_desc,
                    },
                    stdout: None,
                    stderr: None,
                    compile_output: None,
                    time_ms: None,
                    memory_kb: None,
                    exit_code: None,
                });
            }
        }
    }

    Ok((
        axum::http::StatusCode::CREATED,
        Json(BatchSubmissionResponse {
            submissions: responses,
        }),
    ))
}
