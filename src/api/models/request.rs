use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionRequest {
    pub language: String,
    pub source_code: String,
    #[serde(default)]
    pub stdin: String,
    pub cpu_time_limit_ms: Option<u64>,
    pub memory_limit_mb: Option<u64>,
    pub wall_time_limit_ms: Option<u64>,
    pub webhook_url: Option<String>,
}

impl SubmissionRequest {
    pub fn validate(&self, settings: &crate::config::Settings) -> Result<(), crate::api::errors::ApiError> {
        if let Some(ref url_str) = self.webhook_url {
            let parsed = url::Url::parse(url_str).map_err(|e| {
                crate::api::errors::ApiError::BadRequest(format!("invalid webhook_url: {}", e))
            })?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err(crate::api::errors::ApiError::BadRequest(
                    "webhook_url scheme must be http or https".to_string()
                ));
            }
        }
        if let Some(cpu) = self.cpu_time_limit_ms {
            if cpu == 0 {
                return Err(crate::api::errors::ApiError::BadRequest(
                    "cpu_time_limit_ms must be greater than 0".to_string()
                ));
            }
            if cpu > settings.cpu_limit_ms {
                return Err(crate::api::errors::ApiError::BadRequest(
                    format!("cpu_time_limit_ms ({}) cannot exceed server limit of {}", cpu, settings.cpu_limit_ms)
                ));
            }
        }
        if let Some(mem) = self.memory_limit_mb {
            if mem == 0 {
                return Err(crate::api::errors::ApiError::BadRequest(
                    "memory_limit_mb must be greater than 0".to_string()
                ));
            }
            if mem > settings.memory_limit_mb {
                return Err(crate::api::errors::ApiError::BadRequest(
                    format!("memory_limit_mb ({}) cannot exceed server limit of {}", mem, settings.memory_limit_mb)
                ));
            }
        }
        if let Some(wall) = self.wall_time_limit_ms {
            if wall == 0 {
                return Err(crate::api::errors::ApiError::BadRequest(
                    "wall_time_limit_ms must be greater than 0".to_string()
                ));
            }
            if wall > settings.wall_limit_ms {
                return Err(crate::api::errors::ApiError::BadRequest(
                    format!("wall_time_limit_ms ({}) cannot exceed server limit of {}", wall, settings.wall_limit_ms)
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSubmissionRequest {
    pub submissions: Vec<SubmissionRequest>,
}