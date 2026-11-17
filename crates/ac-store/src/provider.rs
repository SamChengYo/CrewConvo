//! Platform ("agent provider") connections and their A2A agents.

use ac_core::{AppResult, new_id};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{DbResultExt, Store};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ProviderConnection {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub catalog_url: String,
    #[serde(skip)]
    pub token_sealed: Option<String>,
    pub external_workspace_id: Option<String>,
    pub external_workspace_name: Option<String>,
    pub group_id: Option<Uuid>,
    pub created_by: Uuid,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// An agent of a connection, as far as syncing is concerned.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProviderAgentRef {
    pub member_id: Uuid,
    pub external_id: String,
    pub active: bool,
}

/// Everything needed to call an agent over A2A.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct A2aTarget {
    pub url: String,
    pub legacy: bool,
    pub external_id: String,
    pub connection_id: Uuid,
    pub token_sealed: Option<String>,
    pub external_workspace_id: Option<String>,
}

const COLS: &str = "id, workspace_id, name, catalog_url, token_sealed, external_workspace_id, \
     external_workspace_name, group_id, created_by, last_sync_at, last_error, created_at";

impl Store {
    pub async fn create_provider_connection(
        &self,
        workspace_id: Uuid,
        name: &str,
        catalog_url: &str,
        token_sealed: Option<&str>,
        created_by: Uuid,
    ) -> AppResult<ProviderConnection> {
        let sql = format!(
            "INSERT INTO provider_connections (id, workspace_id, name, catalog_url, token_sealed, created_by) \
             VALUES ($1,$2,$3,$4,$5,$6) RETURNING {COLS}"
        );
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(new_id())
            .bind(workspace_id)
            .bind(name)
            .bind(catalog_url)
            .bind(token_sealed)
            .bind(created_by)
            .fetch_one(&self.pool)
            .await
            .db()
    }

    pub async fn get_provider_connection(&self, id: Uuid) -> AppResult<Option<ProviderConnection>> {
        let sql = format!("SELECT {COLS} FROM provider_connections WHERE id = $1");
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()
    }

    pub async fn list_provider_connections(&self, workspace_id: Uuid) -> AppResult<Vec<ProviderConnection>> {
        let sql = format!("SELECT {COLS} FROM provider_connections WHERE workspace_id = $1 ORDER BY created_at");
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(workspace_id).fetch_all(&self.pool).await.db()
    }

    pub async fn all_provider_connections(&self) -> AppResult<Vec<ProviderConnection>> {
        let sql = format!("SELECT {COLS} FROM provider_connections ORDER BY created_at");
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await.db()
    }

    pub async fn delete_provider_connection(&self, id: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM provider_connections WHERE id = $1").bind(id).execute(&self.pool).await.db()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_provider_sync(
        &self,
        id: Uuid,
        name: Option<&str>,
        external_workspace_id: Option<&str>,
        external_workspace_name: Option<&str>,
        group_id: Option<Uuid>,
        error: Option<&str>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE provider_connections SET name = COALESCE($2, name), \
             external_workspace_id = COALESCE($3, external_workspace_id), \
             external_workspace_name = COALESCE($4, external_workspace_name), \
             group_id = COALESCE($5, group_id), last_error = $6, \
             last_sync_at = CASE WHEN $6 IS NULL THEN now() ELSE last_sync_at END WHERE id = $1",
        )
        .bind(id)
        .bind(name)
        .bind(external_workspace_id)
        .bind(external_workspace_name)
        .bind(group_id)
        .bind(error)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }

    pub async fn provider_agents(&self, connection_id: Uuid) -> AppResult<Vec<ProviderAgentRef>> {
        sqlx::query_as(
            "SELECT a.member_id, a.external_id, (m.deactivated_at IS NULL) AS active \
             FROM agents a JOIN members m ON m.id = a.member_id \
             WHERE a.provider_connection_id = $1 AND a.external_id IS NOT NULL",
        )
        .bind(connection_id)
        .fetch_all(&self.pool)
        .await
        .db()
    }

    /// Marks a freshly created agent as an A2A agent of a connection (always reachable → online).
    pub async fn set_provider_agent(
        &self,
        member_id: Uuid,
        connection_id: Uuid,
        external_id: &str,
        a2a_url: &str,
        legacy: bool,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE agents SET provider_connection_id = $2, external_id = $3, a2a_url = $4, a2a_legacy = $5, \
             delivery = 'a2a', mention_only = true, \
             status = CASE WHEN status = 'disabled' THEN status ELSE 'online' END WHERE member_id = $1",
        )
        .bind(member_id)
        .bind(connection_id)
        .bind(external_id)
        .bind(a2a_url)
        .bind(legacy)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }

    /// An agent that disappeared from the catalog and came back.
    pub async fn reactivate_provider_agent(&self, member_id: Uuid) -> AppResult<()> {
        let mut tx = self.pool.begin().await.db()?;
        sqlx::query("UPDATE members SET deactivated_at = NULL WHERE id = $1")
            .bind(member_id)
            .execute(&mut *tx)
            .await
            .db()?;
        sqlx::query("UPDATE agents SET status = 'online' WHERE member_id = $1")
            .bind(member_id)
            .execute(&mut *tx)
            .await
            .db()?;
        tx.commit().await.db()?;
        Ok(())
    }

    pub async fn rename_member(&self, member_id: Uuid, display_name: &str) -> AppResult<()> {
        sqlx::query("UPDATE members SET display_name = $2 WHERE id = $1")
            .bind(member_id)
            .bind(display_name)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }

    pub async fn a2a_target(&self, agent_id: Uuid) -> AppResult<Option<A2aTarget>> {
        sqlx::query_as(
            "SELECT a.a2a_url AS url, a.a2a_legacy AS legacy, a.external_id, c.id AS connection_id, c.token_sealed, \
             c.external_workspace_id \
             FROM agents a JOIN provider_connections c ON c.id = a.provider_connection_id \
             WHERE a.member_id = $1 AND a.a2a_url IS NOT NULL AND a.external_id IS NOT NULL",
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await
        .db()
    }

    pub async fn list_a2a_agent_ids(&self) -> AppResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT a.member_id FROM agents a JOIN members m ON m.id = a.member_id \
             WHERE a.delivery = 'a2a' AND m.deactivated_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await
        .db()?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    pub async fn a2a_context(
        &self,
        agent_id: Uuid,
        conversation_id: Uuid,
        thread_key: &str,
    ) -> AppResult<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT context_id FROM a2a_contexts WHERE agent_id = $1 AND conversation_id = $2 AND thread_key = $3",
        )
        .bind(agent_id)
        .bind(conversation_id)
        .bind(thread_key)
        .fetch_optional(&self.pool)
        .await
        .db()?;
        Ok(row.map(|r| r.0))
    }

    pub async fn set_a2a_context(
        &self,
        agent_id: Uuid,
        conversation_id: Uuid,
        thread_key: &str,
        context_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO a2a_contexts (agent_id, conversation_id, thread_key, context_id) VALUES ($1,$2,$3,$4) \
             ON CONFLICT (agent_id, conversation_id, thread_key) DO UPDATE SET context_id = $4, updated_at = now()",
        )
        .bind(agent_id)
        .bind(conversation_id)
        .bind(thread_key)
        .bind(context_id)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }
}
