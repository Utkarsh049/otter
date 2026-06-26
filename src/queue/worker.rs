use std::sync::Arc;
use tokio::sync::Semaphore;
use crate::config::Settings;
use crate::store::memory::SubmissionStore;
use crate::execution::languages::registry::LanguageRegistry;
use crate::execution::engine::Engine;
use crate::execution::limits::Limits;
use crate::api::models::status::StatusCode;
use crate::execution::result::ExecutionStatus;

pub struct Worker {
    semaphore: Arc<Semaphore>,
    store: Arc<SubmissionStore>,
    registry: Arc<LanguageRegistry>,
    max_concurrent: usize,
}

impl Worker {
    pub fn new(settings: &Settings, store: Arc<SubmissionStore>, registry: Arc<LanguageRegistry>) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(settings.max_concurrent)),
            store,
            registry,
            max_concurrent: settings.max_concurrent,
        }
    }

    pub fn in_flight(&self) -> usize {
        self.max_concurrent.saturating_sub(self.semaphore.available_permits())
    }

    pub fn enqueue(
        &self,
        token: String,
        language_id: String,
        source_code: String,
        stdin: String,
        limits: Limits,
    ) {
        let semaphore = self.semaphore.clone();
        let store = self.store.clone();
        let registry = self.registry.clone();
        let language_id_log = language_id.clone();
        let token_log = token.clone();
        
        tokio::spawn(async move {
            let permit = match semaphore.acquire().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("Failed to acquire semaphore permit: {:?}", e);
                    store.update_status(&token, StatusCode::internal_error());
                    return;
                }
            };
            
            store.update_status(&token, StatusCode::processing());
            
            let lang = match registry.get(&language_id) {
                Some(l) => l,
                None => {
                    store.update_status(&token, StatusCode::internal_error());
                    return;
                }
            };
            
            let exec_result = Engine::execute(lang, source_code, stdin, limits).await;
            
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
                        let excerpt = if res.compile_output.len() > 200 {
                            format!("{}... [truncated]", &res.compile_output[..200])
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
    }
}