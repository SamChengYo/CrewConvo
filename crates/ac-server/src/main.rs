use ac_llm::{LlmConfig, LlmRegistry};
use ac_server::{Edition, ServerConfig};
use ac_store::DbConfig;
use tracing_subscriber::EnvFilter;

mod embedded;

fn database_configured() -> bool {
    ["DATABASE_URL", "PG_HOST"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.trim().is_empty()))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
            }
            Err(_) => ctrl_c.await,
        }
    }
    #[cfg(not(unix))]
    ctrl_c.await;
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info")),
        )
        .init();

    let edition = Edition::from_env().map_err(anyhow::Error::msg)?;
    let data_dir = ServerConfig::data_dir_from_env();

    // Database: an external PostgreSQL if one is configured; otherwise the Personal edition runs
    // its own built-in database. The Enterprise edition always needs an external one.
    let (db, embedded) = if database_configured() {
        (DbConfig::from_env(None).map_err(anyhow::Error::msg)?, None)
    } else if edition == Edition::Personal {
        let (pg, url) = embedded::EmbeddedDb::start(&data_dir).await?;
        (DbConfig::Url(url), Some(pg))
    } else {
        anyhow::bail!(
            "The Enterprise edition needs a PostgreSQL database: set DATABASE_URL (or PG_HOST, PG_PORT, PG_USER, \
             PG_PASSWORD, PG_DBNAME) in .env. See docs/configuration.md."
        );
    };

    let mut config = ServerConfig::from_env(db).map_err(anyhow::Error::msg)?;
    if config.master_key.is_none() {
        // Keep stored secrets readable across restarts without asking the user for a key.
        config.master_key = Some(embedded::master_key(&data_dir)?);
    }
    let llm = LlmRegistry::from_config(&LlmConfig::from_env());
    tracing::info!(providers = ?llm.provider_names(), "LLM providers configured");
    let bind = config.bind.clone();
    let app = ac_server::build(config, llm).await?;

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, edition = edition.as_str(), "CrewConvo server listening");
    // On Ctrl+C / SIGTERM: stop accepting requests, but do not wait for agents that keep a
    // connection open (WebSocket, long-poll) — give in-flight requests 3 seconds, then exit.
    let stopping = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify = stopping.clone();
    let serve = axum::serve(listener, app.router).with_graceful_shutdown(async move {
        shutdown_signal().await;
        tracing::info!("shutting down");
        notify.notify_one();
    });
    let served = tokio::select! {
        r = serve => r,
        _ = async {
            stopping.notified().await;
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        } => Ok(()),
    };
    if let Some(pg) = embedded {
        pg.stop().await;
    }
    served?;
    Ok(())
}
