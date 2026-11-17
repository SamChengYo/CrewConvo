//! Agent groups (who an agent can discover).

use ac_core::{AppResult, new_id};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{DbResultExt, Store};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AgentGroup {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    /// Agent member ids.
    pub agent_ids: Vec<Uuid>,
}

const SELECT: &str = "SELECT g.id, g.workspace_id, g.name, g.description, g.created_by, g.created_at, \
     COALESCE(ARRAY(SELECT gm.agent_id FROM agent_group_members gm WHERE gm.group_id = g.id ORDER BY gm.added_at), \
              '{}'::uuid[]) AS agent_ids \
     FROM agent_groups g";

impl Store {
    pub async fn list_agent_groups(&self, workspace_id: Uuid) -> AppResult<Vec<AgentGroup>> {
        let sql = format!("{SELECT} WHERE g.workspace_id = $1 ORDER BY g.name");
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(workspace_id).fetch_all(&self.pool).await.db()
    }

    pub async fn get_agent_group(&self, id: Uuid) -> AppResult<Option<AgentGroup>> {
        let sql = format!("{SELECT} WHERE g.id = $1");
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()
    }

    pub async fn create_agent_group(
        &self,
        workspace_id: Uuid,
        name: &str,
        description: &str,
        created_by: Uuid,
    ) -> AppResult<Uuid> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO agent_groups (id, workspace_id, name, description, created_by) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(id)
        .bind(workspace_id)
        .bind(name)
        .bind(description)
        .bind(created_by)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(id)
    }

    pub async fn update_agent_group(&self, id: Uuid, name: Option<&str>, description: Option<&str>) -> AppResult<()> {
        sqlx::query(
            "UPDATE agent_groups SET name = COALESCE($2, name), description = COALESCE($3, description) WHERE id = $1",
        )
        .bind(id)
        .bind(name)
        .bind(description)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }

    pub async fn delete_agent_group(&self, id: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM agent_groups WHERE id = $1").bind(id).execute(&self.pool).await.db()?;
        Ok(())
    }

    /// Replaces the member list.
    pub async fn set_agent_group_members(&self, id: Uuid, agent_ids: &[Uuid], by: Uuid) -> AppResult<()> {
        let mut tx = self.pool.begin().await.db()?;
        sqlx::query("DELETE FROM agent_group_members WHERE group_id = $1 AND NOT (agent_id = ANY($2))")
            .bind(id)
            .bind(agent_ids)
            .execute(&mut *tx)
            .await
            .db()?;
        sqlx::query(
            "INSERT INTO agent_group_members (group_id, agent_id, added_by) SELECT $1, unnest($2::uuid[]), $3 \
             ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(agent_ids)
        .bind(by)
        .execute(&mut *tx)
        .await
        .db()?;
        tx.commit().await.db()?;
        Ok(())
    }

    /// Agents that `agent_id` can discover: itself, agents sharing a group, and — for agents that do
    /// not come from a platform connection — agents with the same owner.
    pub async fn visible_agent_ids(&self, agent_id: Uuid) -> AppResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT a.member_id FROM agents a \
             WHERE a.member_id = $1 \
                OR (a.provider_connection_id IS NULL AND a.owner_id = \
                    (SELECT owner_id FROM agents WHERE member_id = $1 AND provider_connection_id IS NULL)) \
                OR a.member_id IN (SELECT g2.agent_id FROM agent_group_members g1 \
                                   JOIN agent_group_members g2 ON g2.group_id = g1.group_id \
                                   WHERE g1.agent_id = $1)",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await
        .db()?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }
}
