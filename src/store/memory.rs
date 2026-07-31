use moka::sync::Cache;
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};
use crate::api::models::response::SubmissionResponse;
use crate::api::models::status::StatusCode;
use crate::execution::result::{ExecutionResult, ExecutionStatus};
use tokio::sync::OnceCell;
use redis::aio::MultiplexedConnection;
use std::time::Duration;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DetailedMetrics {
    pub completed_count: usize,
    pub error_count: usize,
    pub total_latency_ms: u64,
    pub status_accepted: usize,
    pub status_compilation_error: usize,
    pub status_time_limit_exceeded: usize,
    pub status_memory_limit_exceeded: usize,
    pub status_runtime_error: usize,
    pub lang_python: usize,
    pub lang_javascript: usize,
    pub lang_c: usize,
    pub lang_cpp: usize,
}

impl Default for DetailedMetrics {
    fn default() -> Self {
        Self {
            completed_count: 0,
            error_count: 0,
            total_latency_ms: 0,
            status_accepted: 0,
            status_compilation_error: 0,
            status_time_limit_exceeded: 0,
            status_memory_limit_exceeded: 0,
            status_runtime_error: 0,
            lang_python: 0,
            lang_javascript: 0,
            lang_c: 0,
            lang_cpp: 0,
        }
    }
}

pub enum SubmissionStoreInner {
    Memory {
        cache: Cache<String, SubmissionResponse>,
        completed_count: AtomicUsize,
        error_count: AtomicUsize,
        total_latency_ms: AtomicU64,
        status_accepted: AtomicUsize,
        status_compilation_error: AtomicUsize,
        status_time_limit_exceeded: AtomicUsize,
        status_memory_limit_exceeded: AtomicUsize,
        status_runtime_error: AtomicUsize,
        lang_python: AtomicUsize,
        lang_javascript: AtomicUsize,
        lang_c: AtomicUsize,
        lang_cpp: AtomicUsize,
    },
    Redis {
        client: redis::Client,
        conn: OnceCell<MultiplexedConnection>,
    },
}

pub struct SubmissionStore {
    inner: SubmissionStoreInner,
}

impl SubmissionStore {
    pub fn new(redis_url: Option<String>) -> Self {
        if let Some(url) = redis_url {
            tracing::info!(redis_url = ?url, "Initializing Redis SubmissionStore");
            let client = redis::Client::open(url).expect("Invalid Redis URL format");
            Self {
                inner: SubmissionStoreInner::Redis {
                    client,
                    conn: OnceCell::new(),
                },
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
                    status_accepted: AtomicUsize::new(0),
                    status_compilation_error: AtomicUsize::new(0),
                    status_time_limit_exceeded: AtomicUsize::new(0),
                    status_memory_limit_exceeded: AtomicUsize::new(0),
                    status_runtime_error: AtomicUsize::new(0),
                    lang_python: AtomicUsize::new(0),
                    lang_javascript: AtomicUsize::new(0),
                    lang_c: AtomicUsize::new(0),
                    lang_cpp: AtomicUsize::new(0),
                },
            }
        }
    }

    async fn get_redis_conn(&self) -> Option<MultiplexedConnection> {
        match &self.inner {
            SubmissionStoreInner::Redis { client, conn } => {
                let connection = conn.get_or_try_init(|| async {
                    client.get_multiplexed_tokio_connection().await
                }).await;
                connection.ok().cloned()
            }
            _ => None,
        }
    }

    pub async fn insert(&self, token: String, status: StatusCode) {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, .. } => {
                cache.insert(
                    token.clone(),
                    SubmissionResponse {
                        token,
                        status,
                        stdout: None,
                        stderr: None,
                        compile_output: None,
                        time_ms: None,
                        memory_kb: None,
                        exit_code: None,
                    },
                );
            }
            SubmissionStoreInner::Redis { .. } => {
                if let Some(conn) = self.get_redis_conn().await {
                    let mut conn = conn.clone();
                    let key = format!("submission:{}", token);
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
                    if let Ok(json) = serde_json::to_string(&response) {
                        let _ = tokio::time::timeout(Duration::from_secs(2), async {
                            use redis::AsyncCommands;
                            let _: Result<(), redis::RedisError> = conn.set_ex(&key, json, 1800).await;
                            // Push to list:recent_submissions list
                            let _: Result<(), redis::RedisError> = conn.lpush("list:recent_submissions", token.clone()).await;
                            // Trim to 1000 items
                            let _: Result<(), redis::RedisError> = conn.ltrim("list:recent_submissions", 0, 999).await;
                        }).await;
                    }
                }
            }
        }
    }

    pub async fn update_status(&self, token: &str, status: StatusCode) {
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
                if let Some(conn) = self.get_redis_conn().await {
                    let mut conn = conn.clone();
                    let key = format!("submission:{}", token);
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        use redis::AsyncCommands;
                        if let Ok(Some(json_str)) = conn.get::<_, Option<String>>(&key).await {
                            if let Ok(mut entry) = serde_json::from_str::<SubmissionResponse>(&json_str) {
                                let old_id = entry.status.id;
                                let new_id = status.id;
                                entry.status = status;
                                
                                if let Ok(new_json) = serde_json::to_string(&entry) {
                                    let _: Result<(), redis::RedisError> = conn.set_ex(&key, new_json, 1800).await;
                                }

                                // Check transition from active (1 or 2) to completed (>= 3)
                                if old_id < 3 && new_id >= 3 {
                                    let _: Result<(), redis::RedisError> = conn.incr("metrics:completed_count", 1).await;
                                    if new_id > 3 {
                                        let _: Result<(), redis::RedisError> = conn.incr("metrics:error_count", 1).await;
                                    }
                                }
                            }
                        }
                    }).await;
                }
            }
        }
    }

    pub async fn update_result(&self, token: &str, result: ExecutionResult, language_id: &str) {
        match &self.inner {
            SubmissionStoreInner::Memory {
                cache,
                completed_count,
                error_count,
                total_latency_ms,
                status_accepted,
                status_compilation_error,
                status_time_limit_exceeded,
                status_memory_limit_exceeded,
                status_runtime_error,
                lang_python,
                lang_javascript,
                lang_c,
                lang_cpp,
            } => {
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

                        // Increment status
                        match result.status {
                            ExecutionStatus::Accepted => { status_accepted.fetch_add(1, Ordering::Relaxed); }
                            ExecutionStatus::CompilationError => { status_compilation_error.fetch_add(1, Ordering::Relaxed); }
                            ExecutionStatus::TimeLimitExceeded => { status_time_limit_exceeded.fetch_add(1, Ordering::Relaxed); }
                            ExecutionStatus::MemoryLimitExceeded => { status_memory_limit_exceeded.fetch_add(1, Ordering::Relaxed); }
                            ExecutionStatus::RuntimeError | ExecutionStatus::InternalError => { status_runtime_error.fetch_add(1, Ordering::Relaxed); }
                        }

                        // Increment language
                        match language_id {
                            "python" => { lang_python.fetch_add(1, Ordering::Relaxed); }
                            "javascript" => { lang_javascript.fetch_add(1, Ordering::Relaxed); }
                            "c" => { lang_c.fetch_add(1, Ordering::Relaxed); }
                            "cpp" => { lang_cpp.fetch_add(1, Ordering::Relaxed); }
                            _ => {}
                        }
                    }
                }
            }
            SubmissionStoreInner::Redis { .. } => {
                if let Some(conn) = self.get_redis_conn().await {
                    let mut conn = conn.clone();
                    let key = format!("submission:{}", token);
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        use redis::AsyncCommands;
                        if let Ok(Some(json_str)) = conn.get::<_, Option<String>>(&key).await {
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
                                    let _: Result<(), redis::RedisError> = conn.set_ex(&key, new_json, 1800).await;
                                }

                                // Check transition from active (1 or 2) to completed (>= 3)
                                if old_id < 3 && new_id >= 3 {
                                    let _: Result<(), redis::RedisError> = conn.incr("metrics:completed_count", 1).await;
                                    if new_id > 3 {
                                        let _: Result<(), redis::RedisError> = conn.incr("metrics:error_count", 1).await;
                                    }
                                    let _: Result<(), redis::RedisError> = conn.incr("metrics:total_latency_ms", result.time_ms).await;

                                    // Increment status
                                    let status_key = match result.status {
                                        ExecutionStatus::Accepted => "metrics:status:accepted",
                                        ExecutionStatus::CompilationError => "metrics:status:compilation_error",
                                        ExecutionStatus::TimeLimitExceeded => "metrics:status:time_limit_exceeded",
                                        ExecutionStatus::MemoryLimitExceeded => "metrics:status:memory_limit_exceeded",
                                        ExecutionStatus::RuntimeError | ExecutionStatus::InternalError => "metrics:status:runtime_error",
                                    };
                                    let _: Result<(), redis::RedisError> = conn.incr(status_key, 1).await;

                                    // Increment language
                                    let lang_key = match language_id {
                                        "python" => Some("metrics:lang:python"),
                                        "javascript" => Some("metrics:lang:javascript"),
                                        "c" => Some("metrics:lang:c"),
                                        "cpp" => Some("metrics:lang:cpp"),
                                        _ => None,
                                    };
                                    if let Some(lk) = lang_key {
                                        let _: Result<(), redis::RedisError> = conn.incr(lk, 1).await;
                                    }
                                }
                            }
                        }
                    }).await;
                }
            }
        }
    }

    pub async fn get(&self, token: &str) -> Option<SubmissionResponse> {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, .. } => cache.get(token),
            SubmissionStoreInner::Redis { .. } => {
                if let Some(conn) = self.get_redis_conn().await {
                    let mut conn = conn.clone();
                    let key = format!("submission:{}", token);
                    let fetch_res = tokio::time::timeout(Duration::from_secs(2), async {
                        use redis::AsyncCommands;
                        if let Ok(Some(json_str)) = conn.get::<_, Option<String>>(&key).await {
                            serde_json::from_str::<SubmissionResponse>(&json_str).ok()
                        } else {
                            None
                        }
                    }).await;
                    fetch_res.unwrap_or(None)
                } else {
                    None
                }
            }
        }
    }

    pub async fn get_all(&self) -> Vec<SubmissionResponse> {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, .. } => {
                cache.iter().map(|(_, val)| val).collect()
            }
            SubmissionStoreInner::Redis { .. } => {
                if let Some(conn) = self.get_redis_conn().await {
                    let mut conn = conn.clone();
                    let fetch_res = tokio::time::timeout(Duration::from_secs(2), async {
                        use redis::AsyncCommands;
                        let tokens: Vec<String> = conn.lrange("list:recent_submissions", 0, -1).await.unwrap_or_default();
                        if tokens.is_empty() {
                            return Vec::new();
                        }
                        
                        let keys: Vec<String> = tokens.iter().map(|t| format!("submission:{}", t)).collect();
                        let json_strs: Vec<Option<String>> = conn.get(&keys).await.unwrap_or_default();
                        
                        let mut submissions = Vec::new();
                        for json_opt in json_strs {
                            if let Some(json_str) = json_opt {
                                if let Ok(response) = serde_json::from_str::<SubmissionResponse>(&json_str) {
                                    submissions.push(response);
                                }
                            }
                        }
                        submissions
                    }).await;
                    fetch_res.unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub async fn get_detailed_metrics(&self) -> DetailedMetrics {
        match &self.inner {
            SubmissionStoreInner::Memory {
                completed_count,
                error_count,
                total_latency_ms,
                status_accepted,
                status_compilation_error,
                status_time_limit_exceeded,
                status_memory_limit_exceeded,
                status_runtime_error,
                lang_python,
                lang_javascript,
                lang_c,
                lang_cpp,
                ..
            } => DetailedMetrics {
                completed_count: completed_count.load(Ordering::Relaxed),
                error_count: error_count.load(Ordering::Relaxed),
                total_latency_ms: total_latency_ms.load(Ordering::Relaxed),
                status_accepted: status_accepted.load(Ordering::Relaxed),
                status_compilation_error: status_compilation_error.load(Ordering::Relaxed),
                status_time_limit_exceeded: status_time_limit_exceeded.load(Ordering::Relaxed),
                status_memory_limit_exceeded: status_memory_limit_exceeded.load(Ordering::Relaxed),
                status_runtime_error: status_runtime_error.load(Ordering::Relaxed),
                lang_python: lang_python.load(Ordering::Relaxed),
                lang_javascript: lang_javascript.load(Ordering::Relaxed),
                lang_c: lang_c.load(Ordering::Relaxed),
                lang_cpp: lang_cpp.load(Ordering::Relaxed),
            },
            SubmissionStoreInner::Redis { .. } => {
                if let Some(conn) = self.get_redis_conn().await {
                    let mut conn = conn.clone();
                    let fetch_res = tokio::time::timeout(Duration::from_secs(2), async {
                        use redis::AsyncCommands;
                        DetailedMetrics {
                            completed_count: conn.get("metrics:completed_count").await.unwrap_or(0),
                            error_count: conn.get("metrics:error_count").await.unwrap_or(0),
                            total_latency_ms: conn.get("metrics:total_latency_ms").await.unwrap_or(0),
                            status_accepted: conn.get("metrics:status:accepted").await.unwrap_or(0),
                            status_compilation_error: conn.get("metrics:status:compilation_error").await.unwrap_or(0),
                            status_time_limit_exceeded: conn.get("metrics:status:time_limit_exceeded").await.unwrap_or(0),
                            status_memory_limit_exceeded: conn.get("metrics:status:memory_limit_exceeded").await.unwrap_or(0),
                            status_runtime_error: conn.get("metrics:status:runtime_error").await.unwrap_or(0),
                            lang_python: conn.get("metrics:lang:python").await.unwrap_or(0),
                            lang_javascript: conn.get("metrics:lang:javascript").await.unwrap_or(0),
                            lang_c: conn.get("metrics:lang:c").await.unwrap_or(0),
                            lang_cpp: conn.get("metrics:lang:cpp").await.unwrap_or(0),
                        }
                    }).await;
                    fetch_res.unwrap_or_default()
                } else {
                    DetailedMetrics::default()
                }
            }
        }
    }

    pub async fn get_metrics(&self) -> (usize, f64, f64) {
        let m = self.get_detailed_metrics().await;
        let error_rate = if m.completed_count > 0 {
            m.error_count as f64 / m.completed_count as f64
        } else {
            0.0
        };
        let avg_latency = if m.completed_count > 0 {
            m.total_latency_ms as f64 / m.completed_count as f64
        } else {
            0.0
        };
        (m.completed_count, error_rate, avg_latency)
    }
}