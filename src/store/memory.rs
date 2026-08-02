use crate::api::models::response::SubmissionResponse;
use crate::api::models::status::StatusCode;
use crate::execution::result::{ExecutionResult, ExecutionStatus};
use moka::sync::Cache;
use redis::aio::MultiplexedConnection;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Redis connection failed")]
    ConnectionFailed,
    #[error("Redis operation timed out")]
    Timeout,
    #[error("Redis operation failed: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

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
        conn: tokio::sync::Mutex<Option<MultiplexedConnection>>,
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
                    conn: tokio::sync::Mutex::new(None),
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
                let mut guard = conn.lock().await;
                if let Some(ref c) = *guard {
                    return Some(c.clone());
                }
                
                let connect_res = tokio::time::timeout(Duration::from_secs(2), async {
                    client.get_multiplexed_tokio_connection().await
                }).await;

                match connect_res {
                    Ok(Ok(c)) => {
                        *guard = Some(c.clone());
                        Some(c)
                    }
                    _ => {
                        tracing::error!("Failed to establish Redis connection within 2s");
                        None
                    }
                }
            }
            _ => None,
        }
    }

    async fn invalidate_conn(&self) {
        if let SubmissionStoreInner::Redis { conn, .. } = &self.inner {
            let mut guard = conn.lock().await;
            *guard = None;
            tracing::info!("Invalidated stale Redis connection");
        }
    }

    pub async fn insert(&self, token: String, status: StatusCode) -> Result<(), StorageError> {
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
                Ok(())
            }
            SubmissionStoreInner::Redis { .. } => {
                let conn = self.get_redis_conn().await.ok_or(StorageError::ConnectionFailed)?;
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
                let json = serde_json::to_string(&response)?;
                let write_res = tokio::time::timeout(Duration::from_secs(2), async {
                    use redis::AsyncCommands;
                    let _: () = conn.set_ex(&key, json, 1800).await?;
                    let _: () = conn.lpush("list:recent_submissions", token.clone()).await?;
                    let _: () = conn.ltrim("list:recent_submissions", 0, 999).await?;
                    Ok::<(), StorageError>(())
                }).await;

                match write_res {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => {
                        self.invalidate_conn().await;
                        Err(e)
                    }
                    Err(_) => {
                        self.invalidate_conn().await;
                        Err(StorageError::Timeout)
                    }
                }
            }
        }
    }

    pub async fn remove(&self, token: &str) -> Result<(), StorageError> {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, .. } => {
                cache.invalidate(token);
                Ok(())
            }
            SubmissionStoreInner::Redis { .. } => {
                let conn = self.get_redis_conn().await.ok_or(StorageError::ConnectionFailed)?;
                let mut conn = conn.clone();
                let key = format!("submission:{}", token);
                let remove_res = tokio::time::timeout(Duration::from_secs(2), async {
                    use redis::AsyncCommands;
                    let _: () = conn.del(&key).await?;
                    let _: () = conn.lrem("list:recent_submissions", 0, token).await?;
                    Ok::<(), StorageError>(())
                }).await;

                match remove_res {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => {
                        self.invalidate_conn().await;
                        Err(e)
                    }
                    Err(_) => {
                        self.invalidate_conn().await;
                        Err(StorageError::Timeout)
                    }
                }
            }
        }
    }

    pub async fn update_status(&self, token: &str, status: StatusCode) -> Result<(), StorageError> {
        match &self.inner {
            SubmissionStoreInner::Memory {
                cache,
                completed_count,
                error_count,
                ..
            } => {
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
                Ok(())
            }
            SubmissionStoreInner::Redis { .. } => {
                let conn = self.get_redis_conn().await.ok_or(StorageError::ConnectionFailed)?;
                let mut conn = conn.clone();
                let key = format!("submission:{}", token);
                
                let write_res = tokio::time::timeout(Duration::from_secs(2), async {
                    use redis::AsyncCommands;
                    if let Some(json_str) = conn.get::<_, Option<String>>(&key).await? {
                        let mut entry: SubmissionResponse = serde_json::from_str(&json_str)?;
                        let old_id = entry.status.id;
                        let new_id = status.id;
                        entry.status = status;

                        let new_json = serde_json::to_string(&entry)?;

                        // Execute updates in a single pipeline
                        let mut pipe = redis::pipe();
                        pipe.set_ex(&key, new_json, 1800);

                        if old_id < 3 && new_id >= 3 {
                            pipe.incr("metrics:completed_count", 1);
                            if new_id > 3 {
                                pipe.incr("metrics:error_count", 1);
                            }
                        }

                        let _: () = pipe.query_async(&mut conn).await?;
                    }
                    Ok::<(), StorageError>(())
                }).await;

                match write_res {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => {
                        self.invalidate_conn().await;
                        Err(e)
                    }
                    Err(_) => {
                        self.invalidate_conn().await;
                        Err(StorageError::Timeout)
                    }
                }
            }
        }
    }

    pub async fn update_result(
        &self,
        token: &str,
        result: ExecutionResult,
        language_id: &str,
    ) -> Result<(), StorageError> {
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
                            ExecutionStatus::Accepted => {
                                status_accepted.fetch_add(1, Ordering::Relaxed);
                            }
                            ExecutionStatus::CompilationError => {
                                status_compilation_error.fetch_add(1, Ordering::Relaxed);
                            }
                            ExecutionStatus::TimeLimitExceeded => {
                                status_time_limit_exceeded.fetch_add(1, Ordering::Relaxed);
                            }
                            ExecutionStatus::MemoryLimitExceeded => {
                                status_memory_limit_exceeded.fetch_add(1, Ordering::Relaxed);
                            }
                            ExecutionStatus::RuntimeError | ExecutionStatus::InternalError => {
                                status_runtime_error.fetch_add(1, Ordering::Relaxed);
                            }
                        }

                        // Increment language
                        match language_id {
                            "python" => {
                                lang_python.fetch_add(1, Ordering::Relaxed);
                            }
                            "javascript" => {
                                lang_javascript.fetch_add(1, Ordering::Relaxed);
                            }
                            "c" => {
                                lang_c.fetch_add(1, Ordering::Relaxed);
                            }
                            "cpp" => {
                                lang_cpp.fetch_add(1, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(())
            }
            SubmissionStoreInner::Redis { .. } => {
                let conn = self
                    .get_redis_conn()
                    .await
                    .ok_or(StorageError::ConnectionFailed)?;
                let mut conn = conn.clone();
                let key = format!("submission:{}", token);

                let write_res = tokio::time::timeout(Duration::from_secs(2), async {
                    use redis::AsyncCommands;
                    if let Some(json_str) = conn.get::<_, Option<String>>(&key).await? {
                        let mut entry: SubmissionResponse = serde_json::from_str(&json_str)?;
                        let old_id = entry.status.id;
                        let status = match result.status {
                            ExecutionStatus::Accepted => StatusCode::accepted(),
                            ExecutionStatus::TimeLimitExceeded => StatusCode::time_limit_exceeded(),
                            ExecutionStatus::MemoryLimitExceeded => {
                                StatusCode::memory_limit_exceeded()
                            }
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

                        let new_json = serde_json::to_string(&entry)?;

                        let mut pipe = redis::pipe();
                        pipe.set_ex(&key, new_json, 1800);

                        if old_id < 3 && new_id >= 3 {
                            pipe.incr("metrics:completed_count", 1);
                            if new_id > 3 {
                                pipe.incr("metrics:error_count", 1);
                            }
                            pipe.incr("metrics:total_latency_ms", result.time_ms);

                            let status_key = match result.status {
                                ExecutionStatus::Accepted => "metrics:status:accepted",
                                ExecutionStatus::CompilationError => {
                                    "metrics:status:compilation_error"
                                }
                                ExecutionStatus::TimeLimitExceeded => {
                                    "metrics:status:time_limit_exceeded"
                                }
                                ExecutionStatus::MemoryLimitExceeded => {
                                    "metrics:status:memory_limit_exceeded"
                                }
                                ExecutionStatus::RuntimeError | ExecutionStatus::InternalError => {
                                    "metrics:status:runtime_error"
                                }
                            };
                            pipe.incr(status_key, 1);

                            let lang_key = match language_id {
                                "python" => Some("metrics:lang:python"),
                                "javascript" => Some("metrics:lang:javascript"),
                                "c" => Some("metrics:lang:c"),
                                "cpp" => Some("metrics:lang:cpp"),
                                _ => None,
                            };
                            if let Some(lk) = lang_key {
                                pipe.incr(lk, 1);
                            }
                        }

                        let _: () = pipe.query_async(&mut conn).await?;
                    }
                    Ok::<(), StorageError>(())
                })
                .await;

                match write_res {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => {
                        self.invalidate_conn().await;
                        Err(e)
                    }
                    Err(_) => {
                        self.invalidate_conn().await;
                        Err(StorageError::Timeout)
                    }
                }
            }
        }
    }

    pub async fn get(&self, token: &str) -> Result<Option<SubmissionResponse>, StorageError> {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, .. } => Ok(cache.get(token)),
            SubmissionStoreInner::Redis { .. } => {
                let conn = self
                    .get_redis_conn()
                    .await
                    .ok_or(StorageError::ConnectionFailed)?;
                let mut conn = conn.clone();
                let key = format!("submission:{}", token);
                let fetch_res = tokio::time::timeout(Duration::from_secs(2), async {
                    use redis::AsyncCommands;
                    let opt_str: Option<String> = conn.get(&key).await?;
                    if let Some(json_str) = opt_str {
                        let resp: SubmissionResponse = serde_json::from_str(&json_str)?;
                        Ok(Some(resp))
                    } else {
                        Ok(None)
                    }
                })
                .await;

                match fetch_res {
                    Ok(Ok(val)) => Ok(val),
                    Ok(Err(e)) => {
                        self.invalidate_conn().await;
                        Err(e)
                    }
                    Err(_) => {
                        self.invalidate_conn().await;
                        Err(StorageError::Timeout)
                    }
                }
            }
        }
    }

    pub async fn get_all(&self) -> Result<Vec<SubmissionResponse>, StorageError> {
        match &self.inner {
            SubmissionStoreInner::Memory { cache, .. } => {
                Ok(cache.iter().map(|(_, val)| val).collect())
            }
            SubmissionStoreInner::Redis { .. } => {
                let conn = self
                    .get_redis_conn()
                    .await
                    .ok_or(StorageError::ConnectionFailed)?;
                let mut conn = conn.clone();
                let fetch_res = tokio::time::timeout(Duration::from_secs(2), async {
                    use redis::AsyncCommands;
                    let tokens: Vec<String> = conn.lrange("list:recent_submissions", 0, -1).await?;
                    let mut keys: Vec<String> = tokens.iter().map(|t| format!("submission:{}", t)).collect();

                    let mut cursor: u64 = 0;
                    let mut scan_keys = Vec::new();
                    loop {
                        let (new_cursor, batch_keys): (u64, Vec<String>) = redis::cmd("SCAN")
                            .arg(cursor)
                            .arg("MATCH")
                            .arg("submission:*")
                            .arg("COUNT")
                            .arg(1000)
                            .query_async(&mut conn)
                            .await?;
                        for k in batch_keys {
                            if !keys.contains(&k) && !scan_keys.contains(&k) {
                                scan_keys.push(k);
                            }
                        }
                        cursor = new_cursor;
                        if cursor == 0 {
                            break;
                        }
                    }
                    keys.extend(scan_keys);

                    if keys.is_empty() {
                        return Ok(Vec::new());
                    }

                    let json_strs: Vec<Option<String>> = conn.mget(&keys).await?;

                    let mut submissions = Vec::new();
                    for json_opt in json_strs {
                        if let Some(json_str) = json_opt {
                            if let Ok(response) =
                                serde_json::from_str::<SubmissionResponse>(&json_str)
                            {
                                submissions.push(response);
                            }
                        }
                    }
                    Ok(submissions)
                })
                .await;

                match fetch_res {
                    Ok(Ok(val)) => Ok(val),
                    Ok(Err(e)) => {
                        self.invalidate_conn().await;
                        Err(e)
                    }
                    Err(_) => {
                        self.invalidate_conn().await;
                        Err(StorageError::Timeout)
                    }
                }
            }
        }
    }

    pub async fn get_detailed_metrics(&self) -> Result<DetailedMetrics, StorageError> {
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
            } => Ok(DetailedMetrics {
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
            }),
            SubmissionStoreInner::Redis { .. } => {
                let conn = self.get_redis_conn().await.ok_or(StorageError::ConnectionFailed)?;
                let mut conn = conn.clone();
                let keys = vec![
                    "metrics:completed_count",
                    "metrics:error_count",
                    "metrics:total_latency_ms",
                    "metrics:status:accepted",
                    "metrics:status:compilation_error",
                    "metrics:status:time_limit_exceeded",
                    "metrics:status:memory_limit_exceeded",
                    "metrics:status:runtime_error",
                    "metrics:lang:python",
                    "metrics:lang:javascript",
                    "metrics:lang:c",
                    "metrics:lang:cpp",
                ];
                let fetch_res = tokio::time::timeout(Duration::from_secs(2), async {
                    use redis::AsyncCommands;
                    let values: Vec<Option<usize>> = conn.mget(&keys).await?;
                    
                    let get_val = |idx: usize| -> usize {
                        values.get(idx).copied().flatten().unwrap_or(0)
                    };
                    
                    Ok(DetailedMetrics {
                        completed_count: get_val(0),
                        error_count: get_val(1),
                        total_latency_ms: get_val(2) as u64,
                        status_accepted: get_val(3),
                        status_compilation_error: get_val(4),
                        status_time_limit_exceeded: get_val(5),
                        status_memory_limit_exceeded: get_val(6),
                        status_runtime_error: get_val(7),
                        lang_python: get_val(8),
                        lang_javascript: get_val(9),
                        lang_c: get_val(10),
                        lang_cpp: get_val(11),
                    })
                }).await;

                match fetch_res {
                    Ok(Ok(val)) => Ok(val),
                    Ok(Err(e)) => {
                        self.invalidate_conn().await;
                        Err(e)
                    }
                    Err(_) => {
                        self.invalidate_conn().await;
                        Err(StorageError::Timeout)
                    }
                }
            }
        }
    }

    pub async fn get_metrics(&self) -> Result<(usize, f64, f64), StorageError> {
        let m = self.get_detailed_metrics().await?;
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
        Ok((m.completed_count, error_rate, avg_latency))
    }
}
