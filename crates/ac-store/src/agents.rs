use ac_core::AppResult;
use ac_core::model::{Agent, AgentOrigin, AgentStatus, MemberKind};
use serde_json::Value;
use sqlx::AssertSqlSafe;
use sqlx::types::Json;
use uuid::Uuid;

use crate::rows::{AGENT_COLS, AgentRow, collect};
use crate::{DbResultExt, Store};

#[derive(Debug, Clone)]
pub struct NewAgent {
    pub workspace_id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub origin: AgentOrigin,
    pub owner_id: Uuid,
    pub description: String,
    pub expertise: Vec<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub tools: Value,
    pub max_wake_seconds: i32,
    pub daily_token_budget: Option<i64>,
    pub sandbox_config: Value,
}

/// Partial update; `None` keeps the current value.
#[derive(Debug, Clone, Default)]
pub struct AgentPatch {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub expertise: Option<Vec<String>>,
    pub status: Option<AgentStatus>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub tools: Option<Value>,
    pub max_wake_seconds: Option<i32>,
    /// `Some(None)` clears the budget.
    pub daily_token_budget: Option<Option<i64>>,
}

impl Store {
    pub async fn create_agent(&self, a: &NewAgent) -> AppResult<Agent> {
        let mut tx = self.pool.begin().await.db()?;
        let member =
            Store::insert_member(&mut tx, a.workspace_id, MemberKind::Agent, &a.handle, &a.display_name, false).await?;
        sqlx::query(
            "INSERT INTO agents (member_id, origin, owner_id, description, expertise, model, system_prompt, tools, \
             max_wake_seconds, daily_token_budget, delivery, sandbox_config) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(member.id)
        .bind(a.origin.as_str())
        .bind(a.owner_id)
        .bind(&a.description)
        .bind(&a.expertise)
        .bind(&a.model)
        .bind(&a.system_prompt)
        .bind(Json(&a.tools))
        .bind(a.max_wake_seconds)
        .bind(a.daily_token_budget)
        .bind(match a.origin {
            AgentOrigin::External => Some("websocket"),
            AgentOrigin::Builtin => None,
        })
        .bind(Json(&a.sandbox_config))
        .execute(&mut *tx)
        .await
        .db()?;
        tx.commit().await.db()?;
        self.get_agent(member.id).await?.ok_or_else(|| ac_core::AppError::internal("agent vanished"))
    }

    pub async fn get_agent(&self, id: Uuid) -> AppResult<Option<Agent>> {
        let sql =
            format!("SELECT {AGENT_COLS} FROM agents a JOIN members m ON m.id = a.member_id WHERE a.member_id = $1");
        let row: Option<AgentRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()?;
        row.map(AgentRow::into_model).transpose()
    }

    pub async fn get_agents(&self, ids: &[Uuid]) -> AppResult<Vec<Agent>> {
        let sql = format!(
            "SELECT {AGENT_COLS} FROM agents a JOIN members m ON m.id = a.member_id WHERE a.member_id = ANY($1) \
             ORDER BY m.handle"
        );
        let rows: Vec<AgentRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(ids).fetch_all(&self.pool).await.db()?;
        collect(rows, AgentRow::into_model)
    }

    /// Agents of a workspace, optionally filtered by a free-text query and/or conversation membership.
    pub async fn list_agents(
        &self,
        workspace_id: Uuid,
        query: Option<&str>,
        conversation_id: Option<Uuid>,
    ) -> AppResult<Vec<Agent>> {
        let sql = format!(
            "SELECT {AGENT_COLS} FROM agents a JOIN members m ON m.id = a.member_id \
             WHERE m.workspace_id = $1 AND m.deactivated_at IS NULL \
             AND ($2::text IS NULL OR m.handle ILIKE '%' || $2 || '%' OR m.display_name ILIKE '%' || $2 || '%' \
                  OR a.description ILIKE '%' || $2 || '%' \
                  OR EXISTS (SELECT 1 FROM unnest(a.expertise) e WHERE e ILIKE '%' || $2 || '%')) \
             AND ($3::uuid IS NULL OR EXISTS (SELECT 1 FROM conversation_members cm \
                  WHERE cm.conversation_id = $3 AND cm.member_id = a.member_id)) \
             ORDER BY m.handle"
        );
        let rows: Vec<AgentRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(workspace_id)
            .bind(query.map(str::trim).filter(|q| !q.is_empty()))
            .bind(conversation_id)
            .fetch_all(&self.pool)
            .await
            .db()?;
        collect(rows, AgentRow::into_model)
    }

    pub async fn list_builtin_agents(&self) -> AppResult<Vec<Agent>> {
        let sql = format!(
            "SELECT {AGENT_COLS} FROM agents a JOIN members m ON m.id = a.member_id \
             WHERE a.origin = 'builtin' AND m.deactivated_at IS NULL"
        );
        let rows: Vec<AgentRow> = sqlx::query_as(AssertSqlSafe(sql)).fetch_all(&self.pool).await.db()?;
        collect(rows, AgentRow::into_model)
    }

    pub async fn update_agent(&self, id: Uuid, p: &AgentPatch) -> AppResult<Agent> {
        let mut tx = self.pool.begin().await.db()?;
        if let Some(name) = &p.display_name {
            sqlx::query("UPDATE members SET display_name = $2 WHERE id = $1")
                .bind(id)
                .bind(name)
                .execute(&mut *tx)
                .await
                .db()?;
        }
        sqlx::query(
            "UPDATE agents SET \
               description = COALESCE($2, description), \
               expertise = COALESCE($3, expertise), \
               status = COALESCE($4, status), \
               model = COALESCE($5, model), \
               system_prompt = COALESCE($6, system_prompt), \
               tools = COALESCE($7, tools), \
               max_wake_seconds = COALESCE($8, max_wake_seconds), \
               daily_token_budget = CASE WHEN $9 THEN $10 ELSE daily_token_budget END \
             WHERE member_id = $1",
        )
        .bind(id)
        .bind(&p.description)
        .bind(&p.expertise)
        .bind(p.status.map(|s| s.as_str()))
        .bind(&p.model)
        .bind(&p.system_prompt)
        .bind(p.tools.as_ref().map(Json))
        .bind(p.max_wake_seconds)
        .bind(p.daily_token_budget.is_some())
        .bind(p.daily_token_budget.flatten())
        .execute(&mut *tx)
        .await
        .db()?;
        tx.commit().await.db()?;
        self.get_agent(id).await?.ok_or_else(|| ac_core::AppError::not_found("agent"))
    }

    /// At startup no external agent is connected yet: connection-based agents start offline
    /// and come back online on their next WebSocket connection or poll. Webhook and A2A agents
    /// are called by the platform, so they are always reachable.
    pub async fn reset_external_presence(&self) -> AppResult<u64> {
        sqlx::query(
            "UPDATE agents a SET status = 'online' FROM members m \
             WHERE m.id = a.member_id AND m.deactivated_at IS NULL AND a.delivery = 'a2a' AND a.status = 'offline'",
        )
        .execute(&self.pool)
        .await
        .db()?;
        let r = sqlx::query(
            "UPDATE agents SET status = 'offline' WHERE origin = 'external' AND status IN ('online', 'busy') \
             AND (delivery IS NULL OR delivery NOT IN ('webhook', 'a2a'))",
        )
        .execute(&self.pool)
        .await
        .db()?;
        Ok(r.rows_affected())
    }

    pub async fn set_agent_status(&self, id: Uuid, status: AgentStatus) -> AppResult<()> {
        sqlx::query("UPDATE agents SET status = $2 WHERE member_id = $1")
            .bind(id)
            .bind(status.as_str())
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }
}

/// Delivery settings of an external agent (secret is stored encrypted by the caller).
#[derive(Debug, Clone)]
pub struct AgentDelivery {
    pub delivery: String,
    pub webhook_url: Option<String>,
    pub webhook_secret_encrypted: Option<String>,
}

impl Store {
    pub async fn set_agent_token(&self, id: Uuid, token_hash: Option<&str>, prefix: Option<&str>) -> AppResult<()> {
        sqlx::query(
            "UPDATE agents SET token_hash = $2, token_prefix = $3, \
             token_created_at = CASE WHEN $2 IS NULL THEN NULL ELSE now() END WHERE member_id = $1",
        )
        .bind(id)
        .bind(token_hash)
        .bind(prefix)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }

    pub async fn agent_by_token_hash(&self, token_hash: &str) -> AppResult<Option<Agent>> {
        let sql = format!(
            "SELECT {AGENT_COLS} FROM agents a JOIN members m ON m.id = a.member_id \
             WHERE a.token_hash = $1 AND m.deactivated_at IS NULL"
        );
        let row: Option<AgentRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(token_hash).fetch_optional(&self.pool).await.db()?;
        row.map(AgentRow::into_model).transpose()
    }

    pub async fn set_agent_delivery(&self, id: Uuid, d: &AgentDelivery) -> AppResult<()> {
        sqlx::query("UPDATE agents SET delivery = $2, webhook_url = $3, webhook_secret = $4 WHERE member_id = $1")
            .bind(id)
            .bind(&d.delivery)
            .bind(&d.webhook_url)
            .bind(&d.webhook_secret_encrypted)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }

    pub async fn agent_delivery(&self, id: Uuid) -> AppResult<Option<AgentDelivery>> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT delivery, webhook_url, webhook_secret FROM agents WHERE member_id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .db()?;
        Ok(row.and_then(|(d, u, s)| {
            d.map(|delivery| AgentDelivery { delivery, webhook_url: u, webhook_secret_encrypted: s })
        }))
    }

    pub async fn touch_agent_seen(&self, id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE agents SET last_seen_at = now() WHERE member_id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }

    pub async fn set_sandbox_config(&self, id: Uuid, cfg: &Value) -> AppResult<()> {
        sqlx::query("UPDATE agents SET sandbox_config = $2 WHERE member_id = $1")
            .bind(id)
            .bind(Json(cfg))
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }

    /// External agents that receive wakes by webhook.
    pub async fn list_webhook_agents(&self) -> AppResult<Vec<Agent>> {
        let sql = format!(
            "SELECT {AGENT_COLS} FROM agents a JOIN members m ON m.id = a.member_id \
             WHERE a.origin = 'external' AND a.delivery = 'webhook' AND m.deactivated_at IS NULL"
        );
        let rows: Vec<AgentRow> = sqlx::query_as(AssertSqlSafe(sql)).fetch_all(&self.pool).await.db()?;
        collect(rows, AgentRow::into_model)
    }

    /// Deletes an agent: deactivates the member, revokes its token, cancels its open wakes
    /// and removes it from conversations. History stays.
    pub async fn deactivate_agent(&self, id: Uuid) -> AppResult<()> {
        let mut tx = self.pool.begin().await.db()?;
        sqlx::query("UPDATE members SET deactivated_at = now() WHERE id = $1 AND deactivated_at IS NULL")
            .bind(id)
            .execute(&mut *tx)
            .await
            .db()?;
        sqlx::query(
            "UPDATE agents SET token_hash = NULL, token_prefix = NULL, status = 'disabled' WHERE member_id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .db()?;
        sqlx::query(
            "UPDATE wakes SET status = 'cancelled', finished_at = now() \
             WHERE agent_id = $1 AND status IN ('pending','delivered','running')",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .db()?;
        sqlx::query("DELETE FROM conversation_members WHERE member_id = $1").bind(id).execute(&mut *tx).await.db()?;
        tx.commit().await.db()?;
        Ok(())
    }
}
