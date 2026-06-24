use std::sync::Arc;
use tokio::sync::Semaphore;
use crate::config::Settings;
use crate::store::memory::SubmissionStore;
use crate::execution::languages::registry::LanguageRegistry;
use crate::execution::engine::Engine;
use crate::execution::limits::Limits;
use crate::api::models::status::StatusCode;

pub struct Worker {
    semaphore: Arc<Semaphore>,
    store: Arc<SubmissionStore>,
    registry: Arc<LanguageRegistry>,
}

impl Worker {
    pub fn new(settings: &Settings, store: Arc<SubmissionStore>, registry: Arc<LanguageRegistry>) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(settings.max_concurrent)),
            store,
            registry,
        }
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
                    store.update_result(&token, res);
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