pub mod errors;
pub mod handlers;
pub mod models;
pub mod routes;

use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use crate::config::Settings;
use crate::execution::languages::registry::LanguageRegistry;
use crate::store::memory::SubmissionStore;
use crate::queue::worker::Worker;

pub async fn serve(settings: Settings) -> Result<()> {
    let registry = Arc::new(LanguageRegistry::build());
    let store    = Arc::new(SubmissionStore::new());
    let worker   = Arc::new(Worker::new(&settings, store.clone(), registry.clone()));
    let worker_shutdown = worker.clone();

    let app = routes::build_router_with_components(settings.clone(), registry, store, worker)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());
    let addr: SocketAddr = format!("{}:{}", settings.host, settings.port).parse()?;
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.unwrap();
            let in_flight = worker_shutdown.in_flight();
            tracing::info!(in_flight = in_flight, "shutting down...");
        })
        .await?;
    Ok(())
}