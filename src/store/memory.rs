use dashmap::DashMap;
use crate::api::models::response::SubmissionResponse;
use crate::api::models::status::StatusCode;

pub struct SubmissionStore {
    inner: DashMap<String, StatusCode>,
}

impl SubmissionStore {
    pub fn new() -> Self {
        Self { inner: DashMap::new() }
    }

    pub fn insert(&self, token: String, status: StatusCode) {
        self.inner.insert(token, status);
    }

    pub fn get(&self, token: &str) -> Option<SubmissionResponse> {
        self.inner.get(token).map(|s| SubmissionResponse {
            token: token.to_string(),
            status: s.clone(),
            stdout: None, stderr: None,
            compile_output: None,
            time_ms: None, memory_kb: None, exit_code: None,
        })
    }
}