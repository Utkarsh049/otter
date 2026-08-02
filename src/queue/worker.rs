use crate::api::models::response::SubmissionResponse;
use crate::api::models::status::StatusCode;
use crate::config::Settings;
use crate::execution::engine::Engine;
use crate::execution::languages::registry::LanguageRegistry;
use crate::execution::limits::Limits;
use crate::execution::result::ExecutionStatus;
use crate::store::memory::SubmissionStore;
use dashmap::DashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

struct QueueDepthGuard(Arc<AtomicUsize>);

impl Drop for QueueDepthGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct QueuedJob {
    pub token: String,
    pub language_id: String,
    pub source_code: String,
    pub stdin: String,
    pub limits: Limits,
    pub ip: IpAddr,
    pub webhook_url: Option<String>,
}
#[derive(Debug)]
pub enum EnqueueError {
    DefinitivelyNotEnqueued(crate::api::errors::ApiError),
    Indeterminate(crate::api::errors::ApiError),
}

pub struct Worker {
    semaphore: Arc<Semaphore>,
    store: Arc<SubmissionStore>,
    registry: Arc<LanguageRegistry>,
    max_concurrent: usize,
    queue_depth: Arc<AtomicUsize>,
    max_queue_depth: usize,
    user_semaphores: Arc<DashMap<IpAddr, Arc<Semaphore>>>,
    max_concurrent_per_ip: usize,
    slots: Arc<super::slot::SlotAllocator>,
    redis_client: Option<redis::Client>,
    conn: tokio::sync::Mutex<Option<redis::aio::MultiplexedConnection>>,
    in_flight: Arc<AtomicUsize>,
    allow_loopback: bool,
}

fn is_blocklisted(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // Loopback: 127.0.0.0/8
            if octets[0] == 127 {
                return true;
            }
            // Private networks:
            // 10.0.0.0/8
            if octets[0] == 10 {
                return true;
            }
            // 172.16.0.0/12 -> 172.16.x.x to 172.31.x.x
            if octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31) {
                return true;
            }
            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            // Link-local: 169.254.0.0/16
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            // Unspecified: 0.0.0.0
            if ipv4.is_unspecified() {
                return true;
            }
            // Multicast: 224.0.0.0/4
            if ipv4.is_multicast() {
                return true;
            }
            // Broadcast: 255.255.255.255
            if octets == [255, 255, 255, 255] {
                return true;
            }

            false
        }
        std::net::IpAddr::V6(ipv6) => {
            let segments = ipv6.segments();
            // Loopback: ::1
            if ipv6.is_loopback() {
                return true;
            }
            // Unspecified: ::
            if ipv6.is_unspecified() {
                return true;
            }
            // Multicast: ff00::/8
            if ipv6.is_multicast() {
                return true;
            }
            // Unique Local: fc00::/7 (fc00:: to fdff::)
            if (segments[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // Link-local: fe80::/10 (fe80:: to febf::)
            if (segments[0] & 0xffc0) == 0xfe80 {
                return true;
            }

            false
        }
    }
}

async fn trigger_webhook(webhook_url: String, response: SubmissionResponse, allow_loopback: bool) {
    let url = match url::Url::parse(&webhook_url) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Invalid webhook URL '{}': {:?}", webhook_url, e);
            return;
        }
    };

    let host = match url.host_str() {
        Some(h) => h,
        None => {
            tracing::error!("No host in webhook URL '{}'", webhook_url);
            return;
        }
    };

    let port = url.port().unwrap_or(match url.scheme() {
        "https" => 443,
        _ => 80,
    });

    // Perform DNS resolution
    let resolved = match tokio::net::lookup_host(format!("{}:{}", host, port)).await {
        Ok(addrs) => addrs,
        Err(e) => {
            tracing::error!("Failed to resolve DNS for host '{}': {:?}", host, e);
            return;
        }
    };

    for addr in resolved {
        let ip = addr.ip();
        if !allow_loopback && is_blocklisted(ip) {
            tracing::warn!(
                "SSRF prevention: blocked webhook request to blocklisted IP {} for URL '{}'",
                ip,
                webhook_url
            );
            return;
        }
    }

    // Send HTTP POST request
    let client = reqwest::Client::new();
    match client.post(&webhook_url).json(&response).send().await {
        Ok(res) => {
            tracing::info!(
                "Webhook sent to '{}' returned status {}",
                webhook_url,
                res.status()
            );
        }
        Err(e) => {
            tracing::error!("Failed to send webhook to '{}': {:?}", webhook_url, e);
        }
    }
}

impl Worker {
    pub fn new(
        settings: &Settings,
        store: Arc<SubmissionStore>,
        registry: Arc<LanguageRegistry>,
    ) -> Self {
        let redis_client = settings
            .redis_url
            .as_ref()
            .map(|url| redis::Client::open(url.clone()).expect("Failed to connect to Redis"));

        let in_flight = Arc::new(AtomicUsize::new(0));
        let allow_loopback = settings.allow_loopback_webhooks;

        if let Some(client) = redis_client.clone() {
            let store = store.clone();
            let registry = registry.clone();
            let slots = Arc::new(super::slot::SlotAllocator::new(settings.max_concurrent));
            let user_semaphores = Arc::new(DashMap::new());
            let max_concurrent_per_ip = settings.max_concurrent_per_ip;
            let in_flight = in_flight.clone();

            // Spawn max_concurrent worker loops
            for _ in 0..settings.max_concurrent {
                let client = client.clone();
                let store = store.clone();
                let registry = registry.clone();
                let slots = slots.clone();
                let user_semaphores = user_semaphores.clone();
                let in_flight = in_flight.clone();

                tokio::spawn(async move {
                    loop {
                        let conn_res = tokio::time::timeout(Duration::from_secs(2), async {
                            client.get_multiplexed_tokio_connection().await
                        }).await;

                        let mut conn = match conn_res {
                            Ok(Ok(c)) => c,
                            Ok(Err(e)) => {
                                tracing::error!("Worker failed to connect to Redis, retrying in 2s: {:?}", e);
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                continue;
                            }
                            Err(_) => {
                                tracing::error!("Worker failed to connect to Redis due to timeout, retrying in 2s");
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                continue;
                            }
                        };

                        loop {
                            use redis::AsyncCommands;
                            let popped: Result<Option<(String, String)>, redis::RedisError> =
                                conn.brpop("queue:submissions", 1.0).await;
                        match popped {
                            Ok(Some((_, json_str))) => {
                                let job: QueuedJob = match serde_json::from_str(&json_str) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::error!("Failed to parse job JSON: {:?}", e);
                                        continue;
                                    }
                                };

                                let ip_sem = user_semaphores
                                    .entry(job.ip)
                                    .or_insert_with(|| {
                                        Arc::new(Semaphore::new(max_concurrent_per_ip))
                                    })
                                    .value()
                                    .clone();

                                let ip_permit = ip_sem.try_acquire();
                                match ip_permit {
                                    Ok(_ip_permit) => {
                                        let _ = store
                                            .update_status(&job.token, StatusCode::processing())
                                            .await;
                                        in_flight.fetch_add(1, Ordering::Relaxed);

                                        let slot_id = match slots.allocate() {
                                            Some(id) => id,
                                            None => {
                                                tracing::error!(
                                                    "No slot available despite thread allocation"
                                                );
                                                let _ = store
                                                    .update_status(
                                                        &job.token,
                                                        StatusCode::internal_error(),
                                                    )
                                                    .await;
                                                if let Some(webhook_url) = job.webhook_url.clone() {
                                                    if let Some(resp) =
                                                        store.get(&job.token).await.ok().flatten()
                                                    {
                                                        tokio::spawn(trigger_webhook(
                                                            webhook_url,
                                                            resp,
                                                            allow_loopback,
                                                        ));
                                                    }
                                                }
                                                in_flight.fetch_sub(1, Ordering::Relaxed);
                                                continue;
                                            }
                                        };

                                        struct SlotGuard {
                                            slot_id: usize,
                                            allocator: Arc<super::slot::SlotAllocator>,
                                        }
                                        impl Drop for SlotGuard {
                                            fn drop(&mut self) {
                                                self.allocator.release(self.slot_id);
                                            }
                                        }
                                        let _slot_guard = SlotGuard {
                                            slot_id,
                                            allocator: slots.clone(),
                                        };

                                        let lang = registry.get(&job.language_id);
                                        if let Some(lang) = lang {
                                            let mut limits_with_slot = job.limits.clone();
                                            limits_with_slot.slot_id = Some(slot_id);

                                            let token_log = job.token.clone();
                                            let language_id_log = job.language_id.clone();

                                            let exec_result = Engine::execute(
                                                lang,
                                                job.source_code,
                                                job.stdin,
                                                limits_with_slot,
                                            )
                                            .await;
                                            match exec_result {
                                                Ok(res) => {
                                                    let _ = store
                                                        .update_result(
                                                            &job.token,
                                                            res.clone(),
                                                            &job.language_id,
                                                        )
                                                        .await;

                                                    let status_str = format!("{:?}", res.status);
                                                    tracing::info!(
                                                        token = %token_log,
                                                        language = %language_id_log,
                                                        status = %status_str,
                                                        cpu_time_ms = res.time_ms,
                                                        memory_kb = res.memory_kb,
                                                        exit_code = res.exit_code,
                                                        "Submission completed"
                                                    );

                                                    if res.status
                                                        == ExecutionStatus::CompilationError
                                                    {
                                                        let excerpt =
                                                            if res.compile_output.chars().count()
                                                                > 200
                                                            {
                                                                let taken: String = res
                                                                    .compile_output
                                                                    .chars()
                                                                    .take(200)
                                                                    .collect();
                                                                format!("{}... [truncated]", taken)
                                                            } else {
                                                                res.compile_output.clone()
                                                            };
                                                        tracing::warn!(
                                                            token = %token_log,
                                                            excerpt = %excerpt,
                                                            "Compilation error occurred"
                                                        );
                                                    }

                                                    match res.status {
                                                        ExecutionStatus::TimeLimitExceeded => {
                                                            tracing::warn!(
                                                                token = %token_log,
                                                                violation_type = "TimeLimitExceeded",
                                                                "Sandbox violation detected"
                                                            );
                                                        }
                                                        ExecutionStatus::MemoryLimitExceeded => {
                                                            tracing::warn!(
                                                                token = %token_log,
                                                                violation_type = "MemoryLimitExceeded",
                                                                "Sandbox violation detected"
                                                            );
                                                        }
                                                        ExecutionStatus::RuntimeError
                                                            if res.exit_code == 128 + 31 =>
                                                        {
                                                            tracing::warn!(
                                                                token = %token_log,
                                                                violation_type = "Seccomp",
                                                                "Sandbox violation detected (blocked system call SIGSYS)"
                                                            );
                                                        }
                                                        _ => {}
                                                    }

                                                    if let Some(webhook_url) =
                                                        job.webhook_url.clone()
                                                    {
                                                        if let Some(resp) = store
                                                            .get(&job.token)
                                                            .await
                                                            .ok()
                                                            .flatten()
                                                        {
                                                            tokio::spawn(trigger_webhook(
                                                                webhook_url,
                                                                resp,
                                                                allow_loopback,
                                                            ));
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::error!(
                                                        "Submission execution failed: {:?}",
                                                        e
                                                    );
                                                    let _ = store
                                                        .update_status(
                                                            &job.token,
                                                            StatusCode::internal_error(),
                                                        )
                                                        .await;
                                                    if let Some(webhook_url) =
                                                        job.webhook_url.clone()
                                                    {
                                                        if let Some(resp) = store
                                                            .get(&job.token)
                                                            .await
                                                            .ok()
                                                            .flatten()
                                                        {
                                                            tokio::spawn(trigger_webhook(
                                                                webhook_url,
                                                                resp,
                                                                allow_loopback,
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            tracing::error!(
                                                "Unsupported language {} for job {}",
                                                job.language_id,
                                                job.token
                                            );
                                            let _ = store
                                                .update_status(
                                                    &job.token,
                                                    StatusCode::internal_error(),
                                                )
                                                .await;
                                            if let Some(webhook_url) = job.webhook_url.clone() {
                                                if let Some(resp) =
                                                    store.get(&job.token).await.ok().flatten()
                                                {
                                                    tokio::spawn(trigger_webhook(
                                                        webhook_url,
                                                        resp,
                                                        allow_loopback,
                                                    ));
                                                }
                                            }
                                        }

                                        in_flight.fetch_sub(1, Ordering::Relaxed);
                                    }
                                    Err(_) => {
                                        // Push job back onto queue (RPUSH) and sleep
                                        let _: Result<(), redis::RedisError> =
                                            conn.rpush("queue:submissions", json_str).await;
                                        tokio::time::sleep(Duration::from_millis(50)).await;
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::error!("BRPOP failed: {:?}. Reconnecting...", e);
                                break;
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
            }
        }

        Self {
            semaphore: Arc::new(Semaphore::new(settings.max_concurrent)),
            store,
            registry,
            max_concurrent: settings.max_concurrent,
            queue_depth: Arc::new(AtomicUsize::new(0)),
            max_queue_depth: settings.max_queue_depth,
            user_semaphores: Arc::new(DashMap::new()),
            max_concurrent_per_ip: settings.max_concurrent_per_ip,
            slots: Arc::new(super::slot::SlotAllocator::new(settings.max_concurrent)),
            redis_client,
            conn: tokio::sync::Mutex::new(None),
            in_flight,
            allow_loopback,
        }
    }

    pub fn in_flight(&self) -> usize {
        if self.redis_client.is_some() {
            self.in_flight.load(Ordering::Relaxed)
        } else {
            self.max_concurrent
                .saturating_sub(self.semaphore.available_permits())
        }
    }

    async fn get_conn(&self) -> Option<redis::aio::MultiplexedConnection> {
        if let Some(ref client) = self.redis_client {
            let mut guard = self.conn.lock().await;
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
                    tracing::error!("Worker failed to establish Redis connection within 2s");
                    None
                }
            }
        } else {
            None
        }
    }

    async fn invalidate_conn(&self) {
        let mut guard = self.conn.lock().await;
        *guard = None;
        tracing::info!("Worker invalidated stale Redis connection");
    }

    pub async fn queue_depth(&self) -> Result<usize, crate::store::memory::StorageError> {
        if self.redis_client.is_some() {
            let depth_res = tokio::time::timeout(Duration::from_secs(2), async {
                let conn = self.get_conn().await.ok_or(crate::store::memory::StorageError::ConnectionFailed)?;
                let mut conn = conn.clone();
                use redis::AsyncCommands;
                let len: usize = conn.llen("queue:submissions").await.map_err(|e| crate::store::memory::StorageError::Redis(e))?;
                Ok::<usize, crate::store::memory::StorageError>(len)
            }).await;

            match depth_res {
                Ok(Ok(d)) => Ok(d),
                Ok(Err(e)) => {
                    self.invalidate_conn().await;
                    Err(e)
                }
                Err(_) => {
                    self.invalidate_conn().await;
                    Err(crate::store::memory::StorageError::Timeout)
                }
            }
        } else {
            Ok(self.queue_depth.load(Ordering::Relaxed))
        }
    }

    pub fn max_queue_depth(&self) -> usize {
        self.max_queue_depth
    }

    async fn check_if_enqueued(&self, token: &str) -> Result<bool, redis::RedisError> {
        let conn = match self.get_conn().await {
            Some(c) => c,
            None => return Err(redis::RedisError::from((redis::ErrorKind::IoError, "Failed to get Redis connection for reconciliation"))),
        };
        let mut conn = conn.clone();
        
        let check_res = tokio::time::timeout(Duration::from_secs(2), async {
            use redis::AsyncCommands;
            let queue: Vec<String> = conn.lrange("queue:submissions", 0, -1).await?;
            for item in queue {
                if let Ok(job) = serde_json::from_str::<QueuedJob>(&item) {
                    if job.token == token {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }).await;

        match check_res {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(redis::RedisError::from((redis::ErrorKind::IoError, "Reconciliation query timed out"))),
        }
    }

    pub async fn enqueue(
        &self,
        token: String,
        language_id: String,
        source_code: String,
        stdin: String,
        limits: Limits,
        ip: IpAddr,
        webhook_url: Option<String>,
    ) -> Result<(), EnqueueError> {
        if self.redis_client.is_some() {
            let conn = match self.get_conn().await {
                Some(c) => c,
                None => {
                    return Err(EnqueueError::DefinitivelyNotEnqueued(
                        crate::api::errors::ApiError::InternalError(
                            "Failed to connect to Redis".to_string(),
                        )
                    ));
                }
            };
            let mut conn = conn.clone();

            let job = QueuedJob {
                token: token.clone(),
                language_id,
                source_code,
                stdin,
                limits,
                ip,
                webhook_url,
            };

            let json_str = serde_json::to_string(&job).map_err(|e| {
                EnqueueError::DefinitivelyNotEnqueued(
                    crate::api::errors::ApiError::InternalError(format!("Failed to serialize job: {}", e))
                )
            })?;

            let script = redis::Script::new(r#"
                local queue = redis.call('LRANGE', KEYS[1], 0, -1)
                for _, v in ipairs(queue) do
                    local decoded = cjson.decode(v)
                    if decoded and decoded.token == ARGV[2] then
                        return 1
                    end
                end
                local len = redis.call('LLEN', KEYS[1])
                if len >= tonumber(ARGV[1]) then
                    return 0
                else
                    redis.call('LPUSH', KEYS[1], ARGV[3])
                    return 1
                end
            "#);

            let res_result = tokio::time::timeout(Duration::from_secs(2), async {
                script.key("queue:submissions")
                    .arg(self.max_queue_depth)
                    .arg(&token)
                    .arg(&json_str)
                    .invoke_async(&mut conn)
                    .await
            }).await;

            let res: i32 = match res_result {
                Ok(Ok(val)) => val,
                Ok(Err(e)) => {
                    self.invalidate_conn().await;
                    match self.check_if_enqueued(&token).await {
                        Ok(true) => 1,
                        Ok(false) => {
                            return Err(EnqueueError::DefinitivelyNotEnqueued(
                                crate::api::errors::ApiError::InternalError(
                                    format!("Redis execution error: {}", e),
                                )
                            ));
                        }
                        Err(rec_err) => {
                            return Err(EnqueueError::Indeterminate(
                                crate::api::errors::ApiError::InternalError(
                                    format!("Redis execution error: {} (Reconciliation failed: {})", e, rec_err),
                                )
                            ));
                        }
                    }
                }
                Err(_) => {
                    self.invalidate_conn().await;
                    match self.check_if_enqueued(&token).await {
                        Ok(true) => 1,
                        Ok(false) => {
                            return Err(EnqueueError::DefinitivelyNotEnqueued(
                                crate::api::errors::ApiError::InternalError(
                                    "Redis timeout during enqueue".to_string(),
                                )
                            ));
                        }
                        Err(rec_err) => {
                            return Err(EnqueueError::Indeterminate(
                                crate::api::errors::ApiError::InternalError(
                                    format!("Redis timeout during enqueue (Reconciliation failed: {})", rec_err),
                                )
                            ));
                        }
                    }
                }
            };

            if res == 0 {
                return Err(EnqueueError::DefinitivelyNotEnqueued(
                    crate::api::errors::ApiError::TooManyRequests(
                        "server is at capacity, try again shortly".to_string(),
                    )
                ));
            }

            Ok(())
        } else {
            // Existing in-memory logic
            let current = self.queue_depth.load(Ordering::Relaxed);
            if current >= self.max_queue_depth {
                return Err(EnqueueError::DefinitivelyNotEnqueued(
                    crate::api::errors::ApiError::TooManyRequests(
                        "server is at capacity, try again shortly".to_string(),
                    )
                ));
            }

            self.queue_depth.fetch_add(1, Ordering::Relaxed);

            let semaphore = self.semaphore.clone();
            let store = self.store.clone();
            let registry = self.registry.clone();
            let language_id_log = language_id.clone();
            let token_log = token.clone();
            let queue_depth = self.queue_depth.clone();

            let slots = self.slots.clone();

            let ip_sem = self
                .user_semaphores
                .entry(ip)
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_concurrent_per_ip)))
                .value()
                .clone();

            let webhook_url_clone = webhook_url.clone();
            let allow_loopback = self.allow_loopback;

            tokio::spawn(async move {
                let _guard = QueueDepthGuard(queue_depth);

                // Acquire IP-specific permit first to avoid global lock contention / HOL blocking
                let _ip_permit = match ip_sem.acquire().await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("Failed to acquire IP semaphore permit: {:?}", e);
                        let _ = store
                            .update_status(&token, StatusCode::internal_error())
                            .await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Ok(Some(resp)) = store.get(&token).await {
                                tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                            }
                        }
                        return;
                    }
                };

                let permit = match semaphore.acquire().await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("Failed to acquire semaphore permit: {:?}", e);
                        let _ = store
                            .update_status(&token, StatusCode::internal_error())
                            .await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Ok(Some(resp)) = store.get(&token).await {
                                tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                            }
                        }
                        return;
                    }
                };

                let slot_id = match slots.allocate() {
                    Some(id) => id,
                    None => {
                        tracing::error!("No free execution slot available despite having permit");
                        let _ = store
                            .update_status(&token, StatusCode::internal_error())
                            .await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Ok(Some(resp)) = store.get(&token).await {
                                tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                            }
                        }
                        return;
                    }
                };

                struct SlotGuard {
                    slot_id: usize,
                    allocator: Arc<crate::queue::slot::SlotAllocator>,
                }
                impl Drop for SlotGuard {
                    fn drop(&mut self) {
                        self.allocator.release(self.slot_id);
                    }
                }
                let _slot_guard = SlotGuard {
                    slot_id,
                    allocator: slots.clone(),
                };

                let _ = store.update_status(&token, StatusCode::processing()).await;

                let lang = match registry.get(&language_id) {
                    Some(l) => l,
                    None => {
                        let _ = store
                            .update_status(&token, StatusCode::internal_error())
                            .await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Ok(Some(resp)) = store.get(&token).await {
                                tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                            }
                        }
                        return;
                    }
                };

                let mut limits_with_slot = limits;
                limits_with_slot.slot_id = Some(slot_id);

                let exec_result = Engine::execute(lang, source_code, stdin, limits_with_slot).await;

                match exec_result {
                    Ok(res) => {
                        let _ = store.update_result(&token, res.clone(), &language_id).await;

                        let status_str = format!("{:?}", res.status);
                        tracing::info!(
                            token = %token_log,
                            language = %language_id_log,
                            status = %status_str,
                            cpu_time_ms = res.time_ms,
                            memory_kb = res.memory_kb,
                            exit_code = res.exit_code,
                            "Submission completed"
                        );

                        if res.status == ExecutionStatus::CompilationError {
                            let excerpt = if res.compile_output.chars().count() > 200 {
                                let taken: String = res.compile_output.chars().take(200).collect();
                                format!("{}... [truncated]", taken)
                            } else {
                                res.compile_output.clone()
                            };
                            tracing::warn!(
                                token = %token_log,
                                excerpt = %excerpt,
                                "Compilation error occurred"
                            );
                        }

                        match res.status {
                            ExecutionStatus::TimeLimitExceeded => {
                                tracing::warn!(
                                    token = %token_log,
                                    violation_type = "TimeLimitExceeded",
                                    "Sandbox violation detected"
                                );
                            }
                            ExecutionStatus::MemoryLimitExceeded => {
                                tracing::warn!(
                                    token = %token_log,
                                    violation_type = "MemoryLimitExceeded",
                                    "Sandbox violation detected"
                                );
                            }
                            ExecutionStatus::RuntimeError if res.exit_code == 128 + 31 => {
                                tracing::warn!(
                                    token = %token_log,
                                    violation_type = "Seccomp",
                                    "Sandbox violation detected (blocked system call SIGSYS)"
                                );
                            }
                            _ => {}
                        }

                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Ok(Some(resp)) = store.get(&token).await {
                                tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Submission execution failed: {:?}", e);
                        let _ = store
                            .update_status(&token, StatusCode::internal_error())
                            .await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Ok(Some(resp)) = store.get(&token).await {
                                tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                            }
                        }
                    }
                }

                drop(permit);
            });
            Ok(())
        }
    }
}
