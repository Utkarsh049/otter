use moka::sync::Cache;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use crate::api::models::response::SubmissionResponse;
use crate::api::models::status::StatusCode;
use crate::execution::result::{ExecutionResult, ExecutionStatus};

pub enum SubmissionStoreInner {
    Memory {
        cache: Cache<String, SubmissionResponse>,
        completed_count: AtomicUsize,
        error_count: AtomicUsize,
        total_latency_ms: AtomicU64,
    },
    Redis {
        client: redis::Client,
    },
}

pub struct SubmissionStore {
    inner: SubmissionStoreInner,
}

impl SubmissionStore {
    pub fn new(redis_url: Option<String>) -> Self {
        if let Some(url) = redis_url {
            tracing::info!(redis_url = ?url, "Initializing Redis SubmissionStore");
            let client = redis::Client::open(url).expect("Failed to connect to Redis");
            Self {
                inner: SubmissionStoreInner::Redis { client },
            }
        } else {
            tracing::info!("Initializing In-Memory SubmissionStore");
            Self {
                inner: SubmissionStoreInner::Memory {
                    cache: Cache::builder()
                        .max_capacity(10_000)
                        .time_to_live(std::time::Duration::from_secs(1800)) // 30 minutes
                        .build(),
                    completed_count: AtomicUsize::new(0),
                    error_count: AtomicUsize::new(0),
                    total_latency_ms: AtomicU64::new(0),
                },
            }
        }
    }

    fn get_redis_conn(&self) -> Option<redis::Connection> {
        match &self.inner {
            SubmissionStoreInner::Redis { client } => client.get_connection().ok(),
            _ => None,
        }
    }

    pub fn insert(&self, token: String, status: StatusCode) {
        let response = SubmissionResponse {
            token: token.clone(),
            status,
            stdout: None,
            stderr: None,
            compile_output: None,
            time_ms: None,
            memory_kb: None,
            exit_code: None,
        };

        match &self.inner {
            SubmissionStoreInner::Memory { cache, .. } => {
                cache.insert(token, response);
            }
            SubmissionStoreInner::Redis { .. } => {
                if let Some(mut conn) = self.get_redis_conn() {
                    use redis::Commands;
                    if let Ok(json) = serde_json::to_string(&response) {
                        let key = format!("submission:{}", token);
                        let _: Result<(), redis::RedisError> = conn.set_ex(&key, json, 1800);
                    }
                }
            }
        }
    }

    pub fn update_status(&self, token: &str, status: StatusCode) {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, completed_count, error_count, .. } => {
                if let Some(mut entry) = cache.get(token) {
                    let old_id = entry.status.id;
                    let new_id = status.id;
                    entry.status = status;
                    
                    cache.insert(token.to_string(), entry);
                    
                    // Check transition from active (1 or 2) to completed (>= 3)
                    if old_id < 3 && new_id >= 3 {
                        completed_count.fetch_add(1, Ordering::Relaxed);
                        if new_id > 3 {
                            error_count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            SubmissionStoreInner::Redis { .. } => {
                if let Some(mut conn) = self.get_redis_conn() {
                    use redis::Commands;
                    let key = format!("submission:{}", token);
                    if let Ok(Some(json_str)) = conn.get::<_, Option<String>>(&key) {
                        if let Ok(mut entry) = serde_json::from_str::<SubmissionResponse>(&json_str) {
                            let old_id = entry.status.id;
                            let new_id = status.id;
                            entry.status = status;
                            
                            if let Ok(new_json) = serde_json::to_string(&entry) {
                                let _: Result<(), redis::RedisError> = conn.set_ex(&key, new_json, 1800);
                            }

                            // Check transition from active (1 or 2) to completed (>= 3)
                            if old_id < 3 && new_id >= 3 {
                                let _: Result<(), redis::RedisError> = conn.incr("metrics:completed_count", 1);
                                if new_id > 3 {
                                    let _: Result<(), redis::RedisError> = conn.incr("metrics:error_count", 1);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn update_result(&self, token: &str, result: ExecutionResult) {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, completed_count, error_count, total_latency_ms } => {
                if let Some(mut entry) = cache.get(token) {
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

                    cache.insert(token.to_string(), entry);

                    // Check transition from active (1 or 2) to completed (>= 3)
                    if old_id < 3 && new_id >= 3 {
                        completed_count.fetch_add(1, Ordering::Relaxed);
                        if new_id > 3 {
                            error_count.fetch_add(1, Ordering::Relaxed);
                        }
                        total_latency_ms.fetch_add(result.time_ms, Ordering::Relaxed);
                    }
                }
            }
            SubmissionStoreInner::Redis { .. } => {
                if let Some(mut conn) = self.get_redis_conn() {
                    use redis::Commands;
                    let key = format!("submission:{}", token);
                    if let Ok(Some(json_str)) = conn.get::<_, Option<String>>(&key) {
                        if let Ok(mut entry) = serde_json::from_str::<SubmissionResponse>(&json_str) {
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

                            if let Ok(new_json) = serde_json::to_string(&entry) {
                                let _: Result<(), redis::RedisError> = conn.set_ex(&key, new_json, 1800);
                            }

                            // Check transition from active (1 or 2) to completed (>= 3)
                            if old_id < 3 && new_id >= 3 {
                                let _: Result<(), redis::RedisError> = conn.incr("metrics:completed_count", 1);
                                if new_id > 3 {
                                    let _: Result<(), redis::RedisError> = conn.incr("metrics:error_count", 1);
                                }
                                let _: Result<(), redis::RedisError> = conn.incr("metrics:total_latency_ms", result.time_ms);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn get(&self, token: &str) -> Option<SubmissionResponse> {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, .. } => cache.get(token),
            SubmissionStoreInner::Redis { .. } => {
                if let Some(mut conn) = self.get_redis_conn() {
                    use redis::Commands;
                    let key = format!("submission:{}", token);
                    if let Ok(Some(json_str)) = conn.get::<_, Option<String>>(&key) {
                        serde_json::from_str::<SubmissionResponse>(&json_str).ok()
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
    }

    pub fn get_metrics(&self) -> (usize, f64, f64) {
        match &self.inner {
            SubmissionStoreInner::Memory { completed_count, error_count, total_latency_ms, .. } => {
                let completed = completed_count.load(Ordering::Relaxed);
                let errors = error_count.load(Ordering::Relaxed);
                let sum_latency = total_latency_ms.load(Ordering::Relaxed);
                
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
            SubmissionStoreInner::Redis { .. } => {
                if let Some(mut conn) = self.get_redis_conn() {
                    use redis::Commands;
                    let completed: usize = conn.get("metrics:completed_count").unwrap_or(0);
                    let errors: usize = conn.get("metrics:error_count").unwrap_or(0);
                    let sum_latency: u64 = conn.get("metrics:total_latency_ms").unwrap_or(0);

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
                } else {
                    (0, 0.0, 0.0)
                }
            }
        }
    }
}