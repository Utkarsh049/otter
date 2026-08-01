pub mod errors;
pub mod handlers;
pub mod json;
pub mod models;
pub mod routes;

pub use json::Json;

use crate::config::Settings;
use crate::execution::languages::registry::LanguageRegistry;
use crate::queue::worker::Worker;
use crate::store::memory::SubmissionStore;
use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub async fn serve(settings: Settings) -> Result<()> {
    let registry = Arc::new(LanguageRegistry::build());
    let store = Arc::new(SubmissionStore::new(settings.redis_url.clone()));
    let worker = Arc::new(Worker::new(&settings, store.clone(), registry.clone()));
    let worker_shutdown = worker.clone();

    let app = routes::build_router_with_components(settings.clone(), registry, store, worker)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());
    let addr: SocketAddr = format!("{}:{}", settings.host, settings.port).parse()?;
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        tokio::signal::ctrl_c().await.unwrap();
        let in_flight = worker_shutdown.in_flight();
        tracing::info!(in_flight = in_flight, "shutting down...");
    })
    .await?;
    Ok(())
}
