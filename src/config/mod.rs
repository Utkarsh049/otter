use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub max_concurrent: usize,
    pub cpu_limit_ms: u64,
    pub wall_limit_ms: u64,
    pub memory_limit_mb: u64,
    pub max_output_bytes: usize,
    pub max_queue_depth: usize,
    pub max_concurrent_per_ip: usize,
    pub redis_url: Option<String>,
    pub rate_limit_requests: Option<u64>,
    pub rate_limit_window_seconds: Option<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            max_concurrent: 8,
            cpu_limit_ms: 5000,
            wall_limit_ms: 10000,
            memory_limit_mb: 128,
            max_output_bytes: 1048576,
            max_queue_depth: 100,
            max_concurrent_per_ip: 2,
            redis_url: None,
            rate_limit_requests: None,
            rate_limit_window_seconds: None,
        }
    }
}

impl Settings {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            host: std::env::var("HOST").unwrap_or("0.0.0.0".into()),
            port: std::env::var("PORT")
                .unwrap_or("8080".into())
                .parse()
                .context("PORT must be a number")?,
            max_concurrent: std::env::var("MAX_CONCURRENT")
                .unwrap_or("8".into())
                .parse()
                .context("MAX_CONCURRENT must be a number")?,
            cpu_limit_ms: std::env::var("CPU_LIMIT_MS")
                .unwrap_or("5000".into())
                .parse()
                .context("CPU_LIMIT_MS must be a number")?,
            wall_limit_ms: std::env::var("WALL_LIMIT_MS")
                .unwrap_or("10000".into())
                .parse()
                .context("WALL_LIMIT_MS must be a number")?,
            memory_limit_mb: std::env::var("MEMORY_LIMIT_MB")
                .unwrap_or("128".into())
                .parse()
                .context("MEMORY_LIMIT_MB must be a number")?,
            max_output_bytes: std::env::var("MAX_OUTPUT_BYTES")
                .unwrap_or("1048576".into())
                .parse()
                .context("MAX_OUTPUT_BYTES must be a number")?,
            max_queue_depth: std::env::var("MAX_QUEUE_DEPTH")
                .unwrap_or("100".into())
                .parse()
                .context("MAX_QUEUE_DEPTH must be a number")?,
            max_concurrent_per_ip: std::env::var("MAX_CONCURRENT_PER_IP")
                .unwrap_or("2".into())
                .parse()
                .context("MAX_CONCURRENT_PER_IP must be a number")?,
            redis_url: std::env::var("REDIS_URL").ok(),
            rate_limit_requests: std::env::var("RATE_LIMIT_REQUESTS").ok()
                .and_then(|s| s.parse().ok()),
            rate_limit_window_seconds: std::env::var("RATE_LIMIT_WINDOW_SECONDS").ok()
                .and_then(|s| s.parse().ok()),
        })
    }
}
