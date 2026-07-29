use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::net::IpAddr;
use dashmap::DashMap;
use tokio::sync::Semaphore;
use crate::config::Settings;
use crate::store::memory::SubmissionStore;
use crate::execution::languages::registry::LanguageRegistry;
use crate::execution::engine::Engine;
use crate::execution::limits::Limits;
use crate::api::models::status::StatusCode;
use crate::execution::result::ExecutionStatus;

struct QueueDepthGuard(Arc<AtomicUsize>);

impl Drop for QueueDepthGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
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
}

impl Worker {
    pub fn new(settings: &Settings, store: Arc<SubmissionStore>, registry: Arc<LanguageRegistry>) -> Self {
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
        }
    }

    pub fn in_flight(&self) -> usize {
        self.max_concurrent.saturating_sub(self.semaphore.available_permits())
    }

    pub fn queue_depth(&self) -> usize {
        self.queue_depth.load(Ordering::Relaxed)
    }

    pub fn max_queue_depth(&self) -> usize {
        self.max_queue_depth
    }

    pub fn enqueue(
        &self,
        token: String,
        language_id: String,
        source_code: String,
        stdin: String,
        limits: Limits,
        ip: IpAddr,
    ) -> Result<(), crate::api::errors::ApiError> {
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

        tokio::spawn(async move {
            let _guard = QueueDepthGuard(queue_depth);
            
            // Acquire IP-specific permit first to avoid global lock contention / HOL blocking
            let _ip_permit = match ip_sem.acquire().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("Failed to acquire IP semaphore permit: {:?}", e);
                    store.update_status(&token, StatusCode::internal_error());
                    return;
                }
            };

            let permit = match semaphore.acquire().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("Failed to acquire semaphore permit: {:?}", e);
                    store.update_status(&token, StatusCode::internal_error());
                    return;
                }
            };
            
            let slot_id = match slots.allocate() {
                Some(id) => id,
                None => {
                    tracing::error!("No free execution slot available despite having permit");
                    store.update_status(&token, StatusCode::internal_error());
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

            store.update_status(&token, StatusCode::processing());
            
            let lang = match registry.get(&language_id) {
                Some(l) => l,
                None => {
                    store.update_status(&token, StatusCode::internal_error());
                    return;
                }
            };
            
            let mut limits_with_slot = limits;
            limits_with_slot.slot_id = Some(slot_id);

            let exec_result = Engine::execute(lang, source_code, stdin, limits_with_slot).await;
            
            match exec_result {
                Ok(res) => {
                    store.update_result(&token, res.clone());
                    
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
                }
                Err(e) => {
                    tracing::error!("Submission execution failed: {:?}", e);
                    store.update_status(&token, StatusCode::internal_error());
                }
            }
            
            drop(permit);
        });
        Ok(())
    }
}