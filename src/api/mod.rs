pub mod errors;
pub mod handlers;
pub mod models;
pub mod routes;

use anyhow::Result;
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use crate::config::Settings;

pub async fn serve(settings: Settings) -> Result<()> {
    let app = routes::build_router(settings.clone())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());
    let addr: SocketAddr = format!("{}:{}", settings.host, settings.port).parse()?;
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.unwrap();
            tracing::info!("shutting down...");
        })
        .await?;
    Ok(())
}