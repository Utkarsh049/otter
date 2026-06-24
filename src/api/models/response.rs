use serde::{Serialize, Deserialize};
use super::status::StatusCode;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubmissionResponse {
    pub token: String,
    pub status: StatusCode,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub compile_output: Option<String>,
    pub time_ms: Option<u64>,
    pub memory_kb: Option<u64>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct LanguageInfo {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}