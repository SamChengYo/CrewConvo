//! Queries used by search indexing, context assembly and summaries.

use ac_core::model::{MessageView, Summary};
use ac_core::{AppResult, new_id};
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use crate::messages::MessageScope;
use crate::rows::{MESSAGE_VIEW_SELECT, MessageViewRow, collect};
use crate::{DbResultExt, Store};

const SCOPE_COND: &str = "(($2::uuid IS NULL AND m.thread_root_id IS NULL) \
     OR ($2::uuid IS NOT NULL AND (m.id = $2 OR m.thread_root_id = $2)))";

fn root(scope: MessageScope) -> Option<Uuid> {
    match scope {
        MessageScope::Main => None,
        MessageScope::Thread(r) => Some(r),
    }
}

impl Store {
    /// Non-deleted, non-system messages ordered by id (UUID v7 ≈ time), for bulk indexing.
    pub async fn messages_for_index(
        &self,
        conversation_id: Option<Uuid>,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> AppResult<Vec<MessageView>> {
        let sql = format!(
            "{MESSAGE_VIEW_SELECT} WHERE m.deleted_at IS NULL AND a.kind <> 'system' \
             AND ($1::uuid IS NULL OR m.conversation_id = $1) AND ($2::uuid IS NULL OR m.id > $2) \
             ORDER BY m.id LIMIT $3"
        );
        let rows: Vec<MessageViewRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(conversation_id)
            .bind(after_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .db()?;
        collect(rows, MessageViewRow::into_model)
    }

    /// The nearest non-deleted message before and after `seq` in the same scope.
    pub async fn message_neighbors(
        &self,
        conversation_id: Uuid,
        scope: MessageScope,
        seq: i64,
    ) -> AppResult<(Option<MessageView>, Option<MessageView>)> {
        let before_sql = format!(
            "{MESSAGE_VIEW_SELECT} WHERE m.conversation_id = $1 AND m.deleted_at IS NULL AND {SCOPE_COND} \
             AND m.seq < $3 ORDER BY m.seq DESC LIMIT 1"
        );
        let after_sql = format!(
            "{MESSAGE_VIEW_SELECT} WHERE m.conversation_id = $1 AND m.deleted_at IS NULL AND {SCOPE_COND} \
             AND m.seq > $3 ORDER BY m.seq ASC LIMIT 1"
        );
        let b: Option<MessageViewRow> = sqlx::query_as(AssertSqlSafe(before_sql))
            .bind(conversation_id)
            .bind(root(scope))
            .bind(seq)
            .fetch_optional(&self.pool)
            .await
            .db()?;
        let a: Option<MessageViewRow> = sqlx::query_as(AssertSqlSafe(after_sql))
            .bind(conversation_id)
            .bind(root(scope))
            .bind(seq)
            .fetch_optional(&self.pool)
            .await
            .db()?;
        Ok((b.map(MessageViewRow::into_model).transpose()?, a.map(MessageViewRow::into_model).transpose()?))
    }

    /// Messages in scope with `after_seq < seq < before_seq`, ascending, at most `limit`.
    pub async fn messages_between(
        &self,
        conversation_id: Uuid,
        scope: MessageScope,
        after_seq: i64,
        before_seq: i64,
        limit: i64,
    ) -> AppResult<Vec<MessageView>> {
        let sql = format!(
            "{MESSAGE_VIEW_SELECT} WHERE m.conversation_id = $1 AND m.deleted_at IS NULL AND {SCOPE_COND} \
             AND m.seq > $3 AND m.seq < $4 ORDER BY m.seq ASC LIMIT $5"
        );
        let rows: Vec<MessageViewRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(conversation_id)
            .bind(root(scope))
            .bind(after_seq)
            .bind(before_seq)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .db()?;
        collect(rows, MessageViewRow::into_model)
    }

    /// seq of the n-th newest non-deleted message in scope (1 = newest), or None if fewer exist.
    pub async fn nth_newest_seq(&self, conversation_id: Uuid, scope: MessageScope, n: i64) -> AppResult<Option<i64>> {
        let sql = format!(
            "SELECT m.seq FROM messages m WHERE m.conversation_id = $1 AND m.deleted_at IS NULL AND {SCOPE_COND} \
             ORDER BY m.seq DESC OFFSET $3 LIMIT 1"
        );
        let row: Option<(i64,)> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(conversation_id)
            .bind(root(scope))
            .bind((n - 1).max(0))
            .fetch_optional(&self.pool)
            .await
            .db()?;
        Ok(row.map(|r| r.0))
    }

    pub async fn latest_summary(
        &self,
        conversation_id: Uuid,
        thread_root_id: Option<Uuid>,
    ) -> AppResult<Option<Summary>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            id: Uuid,
            conversation_id: Uuid,
            thread_root_id: Option<Uuid>,
            covers_up_to_seq: i64,
            body: String,
            token_count: i32,
            created_at: DateTime<Utc>,
        }
        let row: Option<Row> = sqlx::query_as(
            "SELECT id, conversation_id, thread_root_id, covers_up_to_seq, body, token_count, created_at \
             FROM summaries WHERE conversation_id = $1 AND thread_root_id IS NOT DISTINCT FROM $2 \
             ORDER BY covers_up_to_seq DESC, created_at DESC LIMIT 1",
        )
        .bind(conversation_id)
        .bind(thread_root_id)
        .fetch_optional(&self.pool)
        .await
        .db()?;
        Ok(row.map(|r| Summary {
            id: r.id,
            conversation_id: r.conversation_id,
            thread_root_id: r.thread_root_id,
            covers_up_to_seq: r.covers_up_to_seq,
            body: r.body,
            token_count: r.token_count,
            created_at: r.created_at,
        }))
    }

    /// Summaries are append-only (history is kept).
    pub async fn insert_summary(
        &self,
        conversation_id: Uuid,
        thread_root_id: Option<Uuid>,
        covers_up_to_seq: i64,
        body: &str,
        token_count: i32,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO summaries (id, conversation_id, thread_root_id, covers_up_to_seq, body, token_count) \
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(new_id())
        .bind(conversation_id)
        .bind(thread_root_id)
        .bind(covers_up_to_seq)
        .bind(body)
        .bind(token_count)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }
}
