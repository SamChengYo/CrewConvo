//! Starting the whole server from the environment: database, keys, HTTP. Shared by the
//! `crewconvo-server` program and the desktop app.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use ac_llm::{LlmConfig, LlmRegistry};
use ac_store::DbConfig;

use crate::embedded::{self, EmbeddedDb};
use crate::{Edition, ServerConfig};

fn database_configured() -> bool {
    ["DATABASE_URL", "PG_HOST"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.trim().is_empty()))
}

/// Runs the server until `shutdown` completes. `on_ready` is called once it accepts requests,
/// with the address this computer reaches it at (`http://127.0.0.1:<port>`).
pub async fn serve_from_env<S, R>(shutdown: S, on_ready: R) -> anyhow::Result<()>
where
    S: Future<Output = ()> + Send + 'static,
    R: FnOnce(&str),
{
    let edition = Edition::from_env().map_err(anyhow::Error::msg)?;
    let data_dir = ServerConfig::data_dir_from_env();

    // Database: an external PostgreSQL if one is configured; otherwise the Personal edition runs
    // its own built-in database. The Enterprise edition always needs an external one.
    let (db, embedded) = if database_configured() {
        (DbConfig::from_env(None).map_err(anyhow::Error::msg)?, None)
    } else if edition == Edition::Personal {
        let (pg, url) = EmbeddedDb::start(&data_dir).await?;
        (DbConfig::Url(url), Some(pg))
    } else {
        anyhow::bail!(
            "The Enterprise edition needs a PostgreSQL database: set DATABASE_URL (or PG_HOST, PG_PORT, PG_USER, \
             PG_PASSWORD, PG_DBNAME) in .env. See docs/configuration.md."
        );
    };

    let result = serve(edition, db, &data_dir, shutdown, on_ready).await;
    if let Some(pg) = embedded {
        pg.stop().await;
    }
    result
}

async fn serve<S, R>(
    edition: Edition,
    db: DbConfig,
    data_dir: &std::path::Path,
    shutdown: S,
    on_ready: R,
) -> anyhow::Result<()>
where
    S: Future<Output = ()> + Send + 'static,
    R: FnOnce(&str),
{
    let mut config = ServerConfig::from_env(db).map_err(anyhow::Error::msg)?;
    if config.master_key.is_none() {
        // Keep stored secrets readable across restarts without asking the user for a key.
        config.master_key = Some(embedded::master_key(data_dir)?);
    }
    let llm = LlmRegistry::from_config(&LlmConfig::from_env());
    tracing::info!(providers = ?llm.provider_names(), "LLM providers configured");
    let bind = config.bind.clone();
    // Claim the address before the slower startup work, so "already in use" is reported at once.
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|e| anyhow::anyhow!("無法使用 {bind}（可能已經有另一個 CrewConvo 或其他程式在使用）：{e}"))?;
    let app = crate::build(config, llm).await?;

    tracing::info!(%bind, edition = edition.as_str(), "CrewConvo server listening");
    on_ready(&crate::local::local_url(&bind));
    // On shutdown: stop accepting requests, but do not wait for agents that keep a connection
    // open (WebSocket, long-poll) — give in-flight requests 3 seconds, then exit.
    let stopping = Arc::new(tokio::sync::Notify::new());
    let notify = stopping.clone();
    let serve = axum::serve(listener, app.router).with_graceful_shutdown(async move {
        shutdown.await;
        tracing::info!("shutting down");
        notify.notify_one();
    });
    let served = tokio::select! {
        r = serve => r,
        _ = async {
            stopping.notified().await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        } => Ok(()),
    };
    served?;
    Ok(())
}
