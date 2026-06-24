use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or("info".into())
        )
        .init();
    let settings = otter::config::Settings::from_env()?;
    tracing::info!("Otter starting on {}:{}", settings.host, settings.port);
    otter::api::serve(settings).await
}