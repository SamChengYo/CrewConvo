//! PostgreSQL access. The database is the source of truth.

mod agents;
mod auth;
mod connect;
mod conversations;
mod deciders;
mod discussions;
mod groups;
mod history;
mod members;
mod messages;
mod provider;
mod rows;
mod wakes;

pub use agents::{AgentDelivery, AgentPatch, NewAgent};
pub use connect::ConnectRequest;
pub use conversations::ConversationListItem;
pub use deciders::{DeciderConfigRecord, DeciderConfigWrite};
pub use groups::AgentGroup;
pub use messages::{MessageScope, NewMessage};
pub use provider::{A2aTarget, ProviderAgentRef, ProviderConnection};
pub use wakes::{NewWake, WakeActivity};

use ac_core::{AppError, AppResult};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
}

/// Connection settings. Either a full URL or discrete PG_* values (password may contain
/// characters that would need URL escaping).
#[derive(Debug, Clone)]
pub enum DbConfig {
    Url(String),
    Parts { host: String, port: u16, user: String, password: String, dbname: String, ssl: bool },
}

impl DbConfig {
    /// Reads `DATABASE_URL`, or `PG_HOST`/`PG_PORT`/`PG_USER`/`PG_PASSWORD`/`PG_DBNAME`/`PG_SSL`.
    /// `dbname_override` replaces the database name (used by tests).
    pub fn from_env(dbname_override: Option<&str>) -> Result<Self, String> {
        if dbname_override.is_none()
            && let Ok(url) = std::env::var("DATABASE_URL")
        {
            return Ok(DbConfig::Url(url));
        }
        let get = |k: &str| std::env::var(k).map_err(|_| format!("missing env {k}"));
        Ok(DbConfig::Parts {
            host: get("PG_HOST")?,
            port: std::env::var("PG_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(5432),
            user: get("PG_USER")?,
            password: get("PG_PASSWORD")?,
            dbname: match dbname_override {
                Some(d) => d.to_string(),
                None => get("PG_DBNAME")?,
            },
            ssl: std::env::var("PG_SSL")
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "require"))
                .unwrap_or(false),
        })
    }
}

impl Store {
    pub async fn connect(cfg: &DbConfig, max_connections: u32) -> AppResult<Self> {
        let opts = match cfg {
            DbConfig::Url(url) => url.parse::<PgConnectOptions>().map_err(AppError::internal)?,
            DbConfig::Parts { host, port, user, password, dbname, ssl } => PgConnectOptions::new()
                .host(host)
                .port(*port)
                .username(user)
                .password(password)
                .database(dbname)
                .ssl_mode(if *ssl { PgSslMode::Require } else { PgSslMode::Prefer }),
        };
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(15))
            .connect_with(opts)
            .await
            .map_err(AppError::internal)?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> AppResult<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await.map_err(AppError::internal)
    }
}

/// Maps sqlx errors into domain errors.
pub(crate) trait DbResultExt<T> {
    fn db(self) -> AppResult<T>;
}

impl<T> DbResultExt<T> for Result<T, sqlx::Error> {
    fn db(self) -> AppResult<T> {
        self.map_err(|e| {
            if let sqlx::Error::Database(d) = &e {
                if d.is_unique_violation() {
                    return AppError::Conflict(d.message().to_string());
                }
                if d.is_foreign_key_violation() {
                    return AppError::BadRequest(d.message().to_string());
                }
                if d.is_check_violation() {
                    return AppError::BadRequest(d.message().to_string());
                }
            }
            tracing::error!(error = %e, "database error");
            AppError::Internal(e.to_string())
        })
    }
}
