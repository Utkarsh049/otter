use dashmap::DashMap;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use crate::api::models::response::SubmissionResponse;
use crate::api::models::status::StatusCode;
use crate::execution::result::{ExecutionResult, ExecutionStatus};

pub struct SubmissionStore {
    inner: DashMap<String, SubmissionResponse>,
    completed_count: AtomicUsize,
    error_count: AtomicUsize,
    total_latency_ms: AtomicU64,
}

impl SubmissionStore {
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
            completed_count: AtomicUsize::new(0),
            error_count: AtomicUsize::new(0),
            total_latency_ms: AtomicU64::new(0),
        }
    }

    pub fn insert(&self, token: String, status: StatusCode) {
        self.inner.insert(token.clone(), SubmissionResponse {
            token,
            status,
            stdout: None,
            stderr: None,
            compile_output: None,
            time_ms: None,
            memory_kb: None,
            exit_code: None,
        });
    }

    pub fn update_status(&self, token: &str, status: StatusCode) {
        if let Some(mut entry) = self.inner.get_mut(token) {
            let old_id = entry.status.id;
            let new_id = status.id;
            entry.status = status;
            
            // Check transition from active (1 or 2) to completed (>= 3)
            if old_id < 3 && new_id >= 3 {
                self.completed_count.fetch_add(1, Ordering::Relaxed);
                if new_id > 3 {
                    self.error_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn update_result(&self, token: &str, result: ExecutionResult) {
        if let Some(mut entry) = self.inner.get_mut(token) {
            let old_id = entry.status.id;
            let status = match result.status {
                ExecutionStatus::Accepted => StatusCode::accepted(),
                ExecutionStatus::TimeLimitExceeded => StatusCode::time_limit_exceeded(),
                ExecutionStatus::MemoryLimitExceeded => StatusCode::memory_limit_exceeded(),
                ExecutionStatus::CompilationError => StatusCode::compilation_error(),
                ExecutionStatus::RuntimeError => StatusCode::runtime_error(),
                ExecutionStatus::InternalError => StatusCode::internal_error(),
            };
            let new_id = status.id;
            
            entry.status = status;
            entry.stdout = Some(result.stdout);
            entry.stderr = Some(result.stderr);
            entry.compile_output = Some(result.compile_output);
            entry.time_ms = Some(result.time_ms);
            entry.memory_kb = Some(result.memory_kb);
            entry.exit_code = Some(result.exit_code);

            // Check transition from active (1 or 2) to completed (>= 3)
            if old_id < 3 && new_id >= 3 {
                self.completed_count.fetch_add(1, Ordering::Relaxed);
                if new_id > 3 {
                    self.error_count.fetch_add(1, Ordering::Relaxed);
                }
                self.total_latency_ms.fetch_add(result.time_ms, Ordering::Relaxed);
            }
        }
    }

    pub fn get(&self, token: &str) -> Option<SubmissionResponse> {
        self.inner.get(token).map(|s| s.clone())
    }

    pub fn get_metrics(&self) -> (usize, f64, f64) {
        let completed = self.completed_count.load(Ordering::Relaxed);
        let errors = self.error_count.load(Ordering::Relaxed);
        let sum_latency = self.total_latency_ms.load(Ordering::Relaxed);
        
        let error_rate = if completed > 0 {
            errors as f64 / completed as f64
        } else {
            0.0
        };
        
        let avg_latency = if completed > 0 {
            sum_latency as f64 / completed as f64
        } else {
            0.0
        };
        
        (completed, error_rate, avg_latency)
    }
}