use ac_llm::{LlmConfig, LlmRegistry};
use ac_server::ServerConfig;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info")),
        )
        .init();

    let config = ServerConfig::from_env().map_err(anyhow::Error::msg)?;
    let llm = LlmRegistry::from_config(&LlmConfig::from_env());
    tracing::info!(providers = ?llm.provider_names(), "LLM providers configured");
    let bind = config.bind.clone();
    let app = ac_server::build(config, llm).await?;

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "CrewConvo server listening");
    axum::serve(listener, app.router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
