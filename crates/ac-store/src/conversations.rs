use ac_core::model::{AgentStatus, Conversation, ConversationKind, ConversationMemberView, MemberKind, Visibility};
use ac_core::{AppResult, new_id};
use serde::Serialize;
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use crate::rows::{CONV_COLS, ConversationRow, collect};
use crate::{DbResultExt, Store};

#[derive(Debug, Clone, Serialize)]
pub struct ConversationListItem {
    #[serde(flatten)]
    pub conversation: Conversation,
    pub last_read_seq: i64,
    /// Highest seq of a top-level, non-deleted message not authored by the viewer.
    pub last_other_seq: i64,
}

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub async fn create_conversation(
        &self,
        workspace_id: Uuid,
        kind: ConversationKind,
        name: Option<&str>,
        topic: Option<&str>,
        visibility: Visibility,
        created_by: Uuid,
        members: &[Uuid],
    ) -> AppResult<Conversation> {
        let mut tx = self.pool.begin().await.db()?;
        let id = new_id();
        let sql = format!(
            "INSERT INTO conversations AS c (id, workspace_id, kind, name, topic, visibility, created_by) \
             VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING {CONV_COLS}"
        );
        let row: ConversationRow = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(workspace_id)
            .bind(kind.as_str())
            .bind(name)
            .bind(topic)
            .bind(visibility.as_str())
            .bind(created_by)
            .fetch_one(&mut *tx)
            .await
            .db()?;
        for m in members {
            sqlx::query(
                "INSERT INTO conversation_members (conversation_id, member_id, invited_by) VALUES ($1,$2,$3) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(m)
            .bind(if *m == created_by { None } else { Some(created_by) })
            .execute(&mut *tx)
            .await
            .db()?;
        }
        tx.commit().await.db()?;
        row.into_model()
    }

    pub async fn get_conversation(&self, id: Uuid) -> AppResult<Option<Conversation>> {
        let sql = format!("SELECT {CONV_COLS} FROM conversations c WHERE c.id = $1");
        let row: Option<ConversationRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()?;
        row.map(ConversationRow::into_model).transpose()
    }

    /// The DM whose member set is exactly `member_ids`.
    pub async fn find_dm(&self, workspace_id: Uuid, member_ids: &[Uuid]) -> AppResult<Option<Conversation>> {
        let mut ids = member_ids.to_vec();
        ids.sort();
        ids.dedup();
        let sql = format!(
            "SELECT {CONV_COLS} FROM conversations c WHERE c.workspace_id = $1 AND c.kind = 'dm' \
             AND (SELECT array_agg(cm.member_id ORDER BY cm.member_id) FROM conversation_members cm \
                  WHERE cm.conversation_id = c.id) = $2::uuid[] \
             LIMIT 1"
        );
        let row: Option<ConversationRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(workspace_id).bind(&ids).fetch_optional(&self.pool).await.db()?;
        row.map(ConversationRow::into_model).transpose()
    }

    pub async fn list_conversations_for(&self, member_id: Uuid) -> AppResult<Vec<ConversationListItem>> {
        let sql = format!(
            "SELECT {CONV_COLS}, cm.last_read_seq, \
               COALESCE((SELECT max(x.seq) FROM messages x WHERE x.conversation_id = c.id \
                         AND x.thread_root_id IS NULL AND x.deleted_at IS NULL AND x.author_id <> $1), 0) \
                 AS last_other_seq \
             FROM conversation_members cm JOIN conversations c ON c.id = cm.conversation_id \
             WHERE cm.member_id = $1 AND c.archived_at IS NULL \
             ORDER BY c.kind, c.name NULLS LAST, c.created_at"
        );
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            c: ConversationRow,
            last_read_seq: i64,
            last_other_seq: i64,
        }
        let rows: Vec<Row> = sqlx::query_as(AssertSqlSafe(sql)).bind(member_id).fetch_all(&self.pool).await.db()?;
        collect(rows, |r| {
            Ok(ConversationListItem {
                conversation: r.c.into_model()?,
                last_read_seq: r.last_read_seq,
                last_other_seq: r.last_other_seq,
            })
        })
    }

    pub async fn list_public_channels(&self, workspace_id: Uuid) -> AppResult<Vec<Conversation>> {
        let sql = format!(
            "SELECT {CONV_COLS} FROM conversations c WHERE c.workspace_id = $1 AND c.kind = 'channel' \
             AND c.visibility = 'public' AND c.archived_at IS NULL ORDER BY c.name"
        );
        let rows: Vec<ConversationRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(workspace_id).fetch_all(&self.pool).await.db()?;
        collect(rows, ConversationRow::into_model)
    }

    /// Returns true if the member was newly added.
    pub async fn add_conversation_member(
        &self,
        conversation_id: Uuid,
        member_id: Uuid,
        invited_by: Option<Uuid>,
    ) -> AppResult<bool> {
        let r = sqlx::query(
            "INSERT INTO conversation_members (conversation_id, member_id, invited_by) VALUES ($1,$2,$3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(conversation_id)
        .bind(member_id)
        .bind(invited_by)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn remove_conversation_member(&self, conversation_id: Uuid, member_id: Uuid) -> AppResult<bool> {
        let r = sqlx::query("DELETE FROM conversation_members WHERE conversation_id = $1 AND member_id = $2")
            .bind(conversation_id)
            .bind(member_id)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn is_member(&self, conversation_id: Uuid, member_id: Uuid) -> AppResult<bool> {
        let (b,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM conversation_members WHERE conversation_id = $1 AND member_id = $2)",
        )
        .bind(conversation_id)
        .bind(member_id)
        .fetch_one(&self.pool)
        .await
        .db()?;
        Ok(b)
    }

    pub async fn member_conversation_ids(&self, member_id: Uuid) -> AppResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> =
            sqlx::query_as("SELECT conversation_id FROM conversation_members WHERE member_id = $1")
                .bind(member_id)
                .fetch_all(&self.pool)
                .await
                .db()?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    pub async fn conversation_members(&self, conversation_id: Uuid) -> AppResult<Vec<ConversationMemberView>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            member_id: Uuid,
            handle: String,
            display_name: String,
            kind: String,
            description: Option<String>,
            agent_status: Option<String>,
            last_read_seq: i64,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT m.id AS member_id, m.handle, m.display_name, m.kind, a.description, a.status AS agent_status, \
                    cm.last_read_seq \
             FROM conversation_members cm JOIN members m ON m.id = cm.member_id \
             LEFT JOIN agents a ON a.member_id = m.id \
             WHERE cm.conversation_id = $1 AND m.deactivated_at IS NULL ORDER BY m.kind, m.handle",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await
        .db()?;
        collect(rows, |r| {
            Ok(ConversationMemberView {
                member_id: r.member_id,
                handle: r.handle,
                display_name: r.display_name,
                kind: r.kind.parse::<MemberKind>()?,
                description: r.description,
                agent_status: r.agent_status.map(|s| s.parse::<AgentStatus>()).transpose()?,
                last_read_seq: r.last_read_seq,
            })
        })
    }

    pub async fn set_frozen(&self, conversation_id: Uuid, frozen: bool) -> AppResult<Conversation> {
        let sql =
            format!("UPDATE conversations AS c SET frozen_until_human = $2 WHERE c.id = $1 RETURNING {CONV_COLS}");
        let row: ConversationRow =
            sqlx::query_as(AssertSqlSafe(sql)).bind(conversation_id).bind(frozen).fetch_one(&self.pool).await.db()?;
        row.into_model()
    }

    pub async fn update_cursor(&self, conversation_id: Uuid, member_id: Uuid, seq: i64) -> AppResult<()> {
        sqlx::query(
            "UPDATE conversation_members SET last_read_seq = GREATEST(last_read_seq, $3) \
             WHERE conversation_id = $1 AND member_id = $2",
        )
        .bind(conversation_id)
        .bind(member_id)
        .bind(seq)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }

    pub async fn get_cursor(&self, conversation_id: Uuid, member_id: Uuid) -> AppResult<i64> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT last_read_seq FROM conversation_members WHERE conversation_id = $1 AND member_id = $2",
        )
        .bind(conversation_id)
        .bind(member_id)
        .fetch_optional(&self.pool)
        .await
        .db()?;
        Ok(row.map(|r| r.0).unwrap_or(0))
    }

    /// Records a chain-limit notice; returns true only the first time for a chain.
    pub async fn try_insert_chain_notice(&self, chain_root_id: Uuid, reason: &str) -> AppResult<bool> {
        let r = sqlx::query("INSERT INTO chain_notices (chain_root_id, reason) VALUES ($1,$2) ON CONFLICT DO NOTHING")
            .bind(chain_root_id)
            .bind(reason)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(r.rows_affected() > 0)
    }
}
