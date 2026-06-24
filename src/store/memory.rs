use dashmap::DashMap;
use crate::api::models::response::SubmissionResponse;
use crate::api::models::status::StatusCode;
use crate::execution::result::{ExecutionResult, ExecutionStatus};

pub struct SubmissionStore {
    inner: DashMap<String, SubmissionResponse>,
}

impl SubmissionStore {
    pub fn new() -> Self {
        Self { inner: DashMap::new() }
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
            entry.status = status;
        }
    }

    pub fn update_result(&self, token: &str, result: ExecutionResult) {
        if let Some(mut entry) = self.inner.get_mut(token) {
            entry.status = match result.status {
                ExecutionStatus::Accepted => StatusCode::accepted(),
                ExecutionStatus::TimeLimitExceeded => StatusCode::time_limit_exceeded(),
                ExecutionStatus::MemoryLimitExceeded => StatusCode::memory_limit_exceeded(),
                ExecutionStatus::CompilationError => StatusCode::compilation_error(),
                ExecutionStatus::RuntimeError => StatusCode::runtime_error(),
                ExecutionStatus::InternalError => StatusCode::internal_error(),
            };
            entry.stdout = Some(result.stdout);
            entry.stderr = Some(result.stderr);
            entry.compile_output = Some(result.compile_output);
            entry.time_ms = Some(result.time_ms);
            entry.memory_kb = Some(result.memory_kb);
            entry.exit_code = Some(result.exit_code);
        }
    }

    pub fn get(&self, token: &str) -> Option<SubmissionResponse> {
        self.inner.get(token).map(|s| s.clone())
    }
}