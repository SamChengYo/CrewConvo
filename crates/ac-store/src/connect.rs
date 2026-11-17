//! Pending `crewconvo connect` requests (browser approval of a CLI).

use ac_core::{AppResult, new_id};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{DbResultExt, Store};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ConnectRequest {
    pub id: Uuid,
    pub user_code: String,
    pub client: Value,
    pub status: String,
    pub workspace_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    #[serde(skip)]
    pub token_sealed: Option<String>,
    pub decided_by: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

const COLS: &str =
    "id, user_code, client, status, workspace_id, agent_id, token_sealed, decided_by, expires_at, created_at";

impl Store {
    pub async fn create_connect_request(
        &self,
        device_code_hash: &str,
        user_code: &str,
        client: &Value,
        expires_at: DateTime<Utc>,
    ) -> AppResult<ConnectRequest> {
        // Housekeeping: forget requests that expired a day ago.
        sqlx::query("DELETE FROM connect_requests WHERE expires_at < now() - interval '1 day'")
            .execute(&self.pool)
            .await
            .db()?;
        let sql = format!(
            "INSERT INTO connect_requests (id, device_code_hash, user_code, client, expires_at) \
             VALUES ($1,$2,$3,$4,$5) RETURNING {COLS}"
        );
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(new_id())
            .bind(device_code_hash)
            .bind(user_code)
            .bind(client)
            .bind(expires_at)
            .fetch_one(&self.pool)
            .await
            .db()
    }

    pub async fn connect_request_by_user_code(&self, user_code: &str) -> AppResult<Option<ConnectRequest>> {
        let sql = format!("SELECT {COLS} FROM connect_requests WHERE user_code = $1");
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(user_code).fetch_optional(&self.pool).await.db()
    }

    pub async fn connect_request_by_device(&self, device_code_hash: &str) -> AppResult<Option<ConnectRequest>> {
        let sql = format!("SELECT {COLS} FROM connect_requests WHERE device_code_hash = $1");
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(device_code_hash).fetch_optional(&self.pool).await.db()
    }

    /// pending → approved / denied. Returns false if it was no longer pending or has expired.
    pub async fn decide_connect_request(
        &self,
        id: Uuid,
        approved: bool,
        workspace_id: Uuid,
        agent_id: Option<Uuid>,
        token_sealed: Option<&str>,
        decided_by: Uuid,
    ) -> AppResult<bool> {
        let r = sqlx::query(
            "UPDATE connect_requests SET status = $2, workspace_id = $3, agent_id = $4, token_sealed = $5, decided_by = $6 \
             WHERE id = $1 AND status = 'pending' AND expires_at > now()",
        )
        .bind(id)
        .bind(if approved { "approved" } else { "denied" })
        .bind(workspace_id)
        .bind(agent_id)
        .bind(token_sealed)
        .bind(decided_by)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(r.rows_affected() > 0)
    }

    /// approved → consumed (the CLI got its token); returns the sealed token exactly once.
    pub async fn consume_connect_request(&self, id: Uuid) -> AppResult<Option<String>> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "UPDATE connect_requests c SET status = 'consumed', token_sealed = NULL \
             FROM (SELECT id, token_sealed FROM connect_requests WHERE id = $1 AND status = 'approved' FOR UPDATE) old \
             WHERE c.id = old.id RETURNING old.token_sealed",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .db()?;
        Ok(row.and_then(|r| r.0))
    }
}
