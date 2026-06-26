use axum::{Extension, Json};
use std::sync::Arc;
use serde::Serialize;
use crate::store::memory::SubmissionStore;

#[derive(Debug, Serialize)]
pub struct MetricsResponse {
    pub count: usize,
    pub error_rate: f64,
    pub avg_latency: f64,
}

pub async fn get_metrics(
    Extension(store): Extension<Arc<SubmissionStore>>,
) -> Json<MetricsResponse> {
    let (count, error_rate, avg_latency) = store.get_metrics();
    Json(MetricsResponse {
        count,
        error_rate,
        avg_latency,
    })
}
