use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SubmissionRequest {
    pub language: String,
    pub source_code: String,
    #[serde(default)]
    pub stdin: String,
    pub cpu_time_limit_ms: Option<u64>,
    pub memory_limit_mb: Option<u64>,
    pub wall_time_limit_ms: Option<u64>,
}