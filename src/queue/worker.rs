use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::net::IpAddr;
use std::time::Duration;
use dashmap::DashMap;
use tokio::sync::Semaphore;
use crate::config::Settings;
use crate::store::memory::SubmissionStore;
use crate::execution::languages::registry::LanguageRegistry;
use crate::execution::engine::Engine;
use crate::execution::limits::Limits;
use crate::api::models::status::StatusCode;
use crate::execution::result::ExecutionStatus;
use crate::api::models::response::SubmissionResponse;

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
    in_flight: Arc<AtomicUsize>,
    allow_loopback: bool,
}

fn is_blocklisted(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // Loopback: 127.0.0.0/8
            if octets[0] == 127 { return true; }
            // Private networks:
            // 10.0.0.0/8
            if octets[0] == 10 { return true; }
            // 172.16.0.0/12 -> 172.16.x.x to 172.31.x.x
            if octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31) { return true; }
            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 { return true; }
            // Link-local: 169.254.0.0/16
            if octets[0] == 169 && octets[1] == 254 { return true; }
            // Unspecified: 0.0.0.0
            if ipv4.is_unspecified() { return true; }
            // Multicast: 224.0.0.0/4
            if ipv4.is_multicast() { return true; }
            // Broadcast: 255.255.255.255
            if octets == [255, 255, 255, 255] { return true; }
            
            false
        }
        std::net::IpAddr::V6(ipv6) => {
            let segments = ipv6.segments();
            // Loopback: ::1
            if ipv6.is_loopback() { return true; }
            // Unspecified: ::
            if ipv6.is_unspecified() { return true; }
            // Multicast: ff00::/8
            if ipv6.is_multicast() { return true; }
            // Unique Local: fc00::/7 (fc00:: to fdff::)
            if (segments[0] & 0xfe00) == 0xfc00 { return true; }
            // Link-local: fe80::/10 (fe80:: to febf::)
            if (segments[0] & 0xffc0) == 0xfe80 { return true; }
            
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
            tracing::warn!("SSRF prevention: blocked webhook request to blocklisted IP {} for URL '{}'", ip, webhook_url);
            return;
        }
    }

    // Send HTTP POST request
    let client = reqwest::Client::new();
    match client.post(&webhook_url).json(&response).send().await {
        Ok(res) => {
            tracing::info!("Webhook sent to '{}' returned status {}", webhook_url, res.status());
        }
        Err(e) => {
            tracing::error!("Failed to send webhook to '{}': {:?}", webhook_url, e);
        }
    }
}

impl Worker {
    pub fn new(settings: &Settings, store: Arc<SubmissionStore>, registry: Arc<LanguageRegistry>) -> Self {
        let redis_client = settings.redis_url.as_ref().map(|url| {
            redis::Client::open(url.clone()).expect("Failed to connect to Redis")
        });

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
                    let mut conn = match client.get_async_connection().await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!("Worker failed to connect to Redis: {:?}", e);
                            return;
                        }
                    };

                    loop {
                        use redis::AsyncCommands;
                        let popped: Result<Option<(String, String)>, redis::RedisError> = conn.brpop("queue:submissions", 1.0).await;
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
                                    .or_insert_with(|| Arc::new(Semaphore::new(max_concurrent_per_ip)))
                                    .value()
                                    .clone();

                                let ip_permit = ip_sem.try_acquire();
                                match ip_permit {
                                    Ok(_ip_permit) => {
                                        store.update_status(&job.token, StatusCode::processing()).await;
                                        in_flight.fetch_add(1, Ordering::Relaxed);

                                        let slot_id = match slots.allocate() {
                                            Some(id) => id,
                                            None => {
                                                tracing::error!("No slot available despite thread allocation");
                                                store.update_status(&job.token, StatusCode::internal_error()).await;
                                                if let Some(webhook_url) = job.webhook_url.clone() {
                                                    if let Some(resp) = store.get(&job.token).await {
                                                        tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
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

                                            let exec_result = Engine::execute(lang, job.source_code, job.stdin, limits_with_slot).await;
                                            match exec_result {
                                                Ok(res) => {
                                                    store.update_result(&job.token, res.clone(), &job.language_id).await;
                                                    
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

                                                    if let Some(webhook_url) = job.webhook_url.clone() {
                                                        if let Some(resp) = store.get(&job.token).await {
                                                            tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::error!("Submission execution failed: {:?}", e);
                                                    store.update_status(&job.token, StatusCode::internal_error()).await;
                                                    if let Some(webhook_url) = job.webhook_url.clone() {
                                                        if let Some(resp) = store.get(&job.token).await {
                                                            tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                                                        }
                                                    }
                                                }
                                            }
                                        } else {
                                            tracing::error!("Unsupported language {} for job {}", job.language_id, job.token);
                                            store.update_status(&job.token, StatusCode::internal_error()).await;
                                            if let Some(webhook_url) = job.webhook_url.clone() {
                                                if let Some(resp) = store.get(&job.token).await {
                                                    tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                                                }
                                            }
                                        }

                                        in_flight.fetch_sub(1, Ordering::Relaxed);
                                    }
                                    Err(_) => {
                                        // Push job back onto queue (RPUSH) and sleep
                                        let _: Result<(), redis::RedisError> = conn.rpush("queue:submissions", json_str).await;
                                        tokio::time::sleep(Duration::from_millis(50)).await;
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::error!("BRPOP failed: {:?}", e);
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
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
            in_flight,
            allow_loopback,
        }
    }

    pub fn in_flight(&self) -> usize {
        if self.redis_client.is_some() {
            self.in_flight.load(Ordering::Relaxed)
        } else {
            self.max_concurrent.saturating_sub(self.semaphore.available_permits())
        }
    }

    pub async fn queue_depth(&self) -> usize {
        if let Some(ref client) = self.redis_client {
            if let Ok(mut conn) = client.get_async_connection().await {
                use redis::AsyncCommands;
                conn.llen("queue:submissions").await.unwrap_or(0)
            } else {
                0
            }
        } else {
            self.queue_depth.load(Ordering::Relaxed)
        }
    }

    pub fn max_queue_depth(&self) -> usize {
        self.max_queue_depth
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
    ) -> Result<(), crate::api::errors::ApiError> {
        if let Some(ref client) = self.redis_client {
            let mut conn = client.get_async_connection().await.map_err(|_e| {
                crate::api::errors::ApiError::InternalError("Failed to connect to Redis".to_string())
            })?;

            use redis::AsyncCommands;
            let current: usize = conn.llen("queue:submissions").await.unwrap_or(0);
            if current >= self.max_queue_depth {
                return Err(crate::api::errors::ApiError::TooManyRequests(
                    "server is at capacity, try again shortly".to_string()
                ));
            }

            let job = QueuedJob {
                token,
                language_id,
                source_code,
                stdin,
                limits,
                ip,
                webhook_url,
            };

            let json = serde_json::to_string(&job).map_err(|_e| {
                crate::api::errors::ApiError::InternalError("Failed to serialize job".to_string())
            })?;

            conn.lpush::<_, _, ()>("queue:submissions", json).await.map_err(|e| {
                crate::api::errors::ApiError::InternalError(format!("Failed to enqueue job in Redis: {:?}", e))
            })?;
            Ok(())
        } else {
            // Existing in-memory logic
            let current = self.queue_depth.load(Ordering::Relaxed);
            if current >= self.max_queue_depth {
                return Err(crate::api::errors::ApiError::TooManyRequests(
                    "server is at capacity, try again shortly".to_string()
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
            
            let ip_sem = self.user_semaphores
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
                        store.update_status(&token, StatusCode::internal_error()).await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Some(resp) = store.get(&token).await {
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
                        store.update_status(&token, StatusCode::internal_error()).await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Some(resp) = store.get(&token).await {
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
                        store.update_status(&token, StatusCode::internal_error()).await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Some(resp) = store.get(&token).await {
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

                store.update_status(&token, StatusCode::processing()).await;
                
                let lang = match registry.get(&language_id) {
                    Some(l) => l,
                    None => {
                        store.update_status(&token, StatusCode::internal_error()).await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Some(resp) = store.get(&token).await {
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
                        store.update_result(&token, res.clone(), &language_id).await;
                        
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
                            if let Some(resp) = store.get(&token).await {
                                tokio::spawn(trigger_webhook(webhook_url, resp, allow_loopback));
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Submission execution failed: {:?}", e);
                        store.update_status(&token, StatusCode::internal_error()).await;
                        if let Some(webhook_url) = webhook_url_clone.clone() {
                            if let Some(resp) = store.get(&token).await {
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