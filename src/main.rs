use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    
    let env_log_format = std::env::var("LOG_FORMAT").unwrap_or_default();
    let is_production = std::env::var("APP_ENV").unwrap_or_default() == "production" 
        || env_log_format == "json";

    if is_production {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(
                std::env::var("RUST_LOG").unwrap_or("info".into())
            )
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                std::env::var("RUST_LOG").unwrap_or("info".into())
            )
            .init();
    }

    let settings = otter::config::Settings::from_env()?;
    tracing::info!(
        host = %settings.host,
        port = settings.port,
        max_concurrent = settings.max_concurrent,
        cpu_limit_ms = settings.cpu_limit_ms,
        wall_limit_ms = settings.wall_limit_ms,
        memory_limit_mb = settings.memory_limit_mb,
        max_output_bytes = settings.max_output_bytes,
        redis_url = ?settings.redis_url,
        "Otter starting with configuration"
    );
    otter::api::serve(settings).await
}