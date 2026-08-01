use crate::api::errors::ApiError;
use crate::api::Json;
use crate::queue::worker::Worker;
use crate::store::memory::SubmissionStore;
use axum::Extension;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct SubmissionsMetrics {
    pub count: usize,
    pub error_rate: f64,
    pub avg_latency_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct StatusBreakdown {
    pub accepted: usize,
    pub compilation_error: usize,
    pub time_limit_exceeded: usize,
    pub memory_limit_exceeded: usize,
    pub runtime_error: usize,
}

#[derive(Debug, Serialize)]
pub struct LanguageMetrics {
    pub python: usize,
    pub javascript: usize,
    pub c: usize,
    pub cpp: usize,
}

#[derive(Debug, Serialize)]
pub struct QueueMetrics {
    pub depth: usize,
    pub in_flight: usize,
}

#[derive(Debug, Serialize)]
pub struct MetricsResponse {
    pub submissions: SubmissionsMetrics,
    pub status_breakdown: StatusBreakdown,
    pub languages: LanguageMetrics,
    pub queue: QueueMetrics,
}

pub async fn get_metrics(
    Extension(store): Extension<Arc<SubmissionStore>>,
    Extension(worker): Extension<Arc<Worker>>,
) -> Result<Json<MetricsResponse>, ApiError> {
    let (detailed, depth) = tokio::try_join!(
        store.get_detailed_metrics(),
        worker.queue_depth()
    ).map_err(|e| {
        ApiError::InternalError(format!("Failed to retrieve metrics: {}", e))
    })?;

    let error_rate = if detailed.completed_count > 0 {
        detailed.error_count as f64 / detailed.completed_count as f64
    } else {
        0.0
    };

    let avg_latency_ms = if detailed.completed_count > 0 {
        detailed.total_latency_ms as f64 / detailed.completed_count as f64
    } else {
        0.0
    };

    Ok(Json(MetricsResponse {
        submissions: SubmissionsMetrics {
            count: detailed.completed_count,
            error_rate,
            avg_latency_ms,
        },
        status_breakdown: StatusBreakdown {
            accepted: detailed.status_accepted,
            compilation_error: detailed.status_compilation_error,
            time_limit_exceeded: detailed.status_time_limit_exceeded,
            memory_limit_exceeded: detailed.status_memory_limit_exceeded,
            runtime_error: detailed.status_runtime_error,
        },
        languages: LanguageMetrics {
            python: detailed.lang_python,
            javascript: detailed.lang_javascript,
            c: detailed.lang_c,
            cpp: detailed.lang_cpp,
        },
        queue: QueueMetrics {
            depth,
            in_flight: worker.in_flight(),
        },
    }))
}
