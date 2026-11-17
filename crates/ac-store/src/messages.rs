use ac_core::model::{Message, MessageView, ReactionSummary};
use ac_core::{AppError, AppResult, new_id};
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use sqlx::types::Json;
use uuid::Uuid;

use crate::rows::{MESSAGE_VIEW_SELECT, MessageRow, MessageViewRow, collect};
use crate::{DbResultExt, Store};

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub conversation_id: Uuid,
    pub thread_root_id: Option<Uuid>,
    pub author_id: Uuid,
    pub body: String,
    pub mentions: Vec<Uuid>,
    pub depth: i32,
    /// None → the message is its own chain root.
    pub chain_root_id: Option<Uuid>,
    pub wake_id: Option<Uuid>,
}

/// Which messages of a conversation: the main timeline (top-level only) or one thread
/// (root plus replies).
#[derive(Debug, Clone, Copy)]
pub enum MessageScope {
    Main,
    Thread(Uuid),
}

impl MessageScope {
    pub fn from_thread(root: Option<Uuid>) -> Self {
        match root {
            Some(r) => MessageScope::Thread(r),
            None => MessageScope::Main,
        }
    }
    fn root(&self) -> Option<Uuid> {
        match self {
            MessageScope::Main => None,
            MessageScope::Thread(r) => Some(*r),
        }
    }
}

const SCOPE_COND: &str = "(($2::uuid IS NULL AND m.thread_root_id IS NULL) \
     OR ($2::uuid IS NOT NULL AND (m.id = $2 OR m.thread_root_id = $2)))";

impl Store {
    /// Inserts a message, allocating the next `seq` of the conversation in the same transaction.
    pub async fn insert_message(&self, n: &NewMessage) -> AppResult<Message> {
        let mut tx = self.pool.begin().await.db()?;
        let seq: Option<(i64,)> =
            sqlx::query_as("UPDATE conversations SET next_seq = next_seq + 1 WHERE id = $1 RETURNING next_seq - 1")
                .bind(n.conversation_id)
                .fetch_optional(&mut *tx)
                .await
                .db()?;
        let Some((seq,)) = seq else {
            return Err(AppError::not_found("conversation"));
        };
        let id = new_id();
        let row: MessageRow = sqlx::query_as(
            "INSERT INTO messages (id, conversation_id, seq, thread_root_id, author_id, body, mentions, depth, \
             chain_root_id, wake_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) \
             RETURNING id, conversation_id, seq, thread_root_id, author_id, body, mentions, depth, chain_root_id, \
             wake_id, created_at, edited_at, deleted_at",
        )
        .bind(id)
        .bind(n.conversation_id)
        .bind(seq)
        .bind(n.thread_root_id)
        .bind(n.author_id)
        .bind(&n.body)
        .bind(&n.mentions)
        .bind(n.depth)
        .bind(n.chain_root_id.unwrap_or(id))
        .bind(n.wake_id)
        .fetch_one(&mut *tx)
        .await
        .db()?;
        tx.commit().await.db()?;
        Ok(row.into())
    }

    pub async fn get_message(&self, id: Uuid) -> AppResult<Option<Message>> {
        let row: Option<MessageRow> = sqlx::query_as(
            "SELECT id, conversation_id, seq, thread_root_id, author_id, body, mentions, depth, chain_root_id, \
             wake_id, created_at, edited_at, deleted_at FROM messages WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .db()?;
        Ok(row.map(Into::into))
    }

    /// Non-deleted message as a view.
    pub async fn get_message_view(&self, id: Uuid) -> AppResult<Option<MessageView>> {
        let sql = format!("{MESSAGE_VIEW_SELECT} WHERE m.id = $1 AND m.deleted_at IS NULL");
        let row: Option<MessageViewRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()?;
        row.map(MessageViewRow::into_model).transpose()
    }

    /// Page of messages in ascending seq order, newest `limit` before `before_seq`.
    pub async fn list_messages(
        &self,
        conversation_id: Uuid,
        scope: MessageScope,
        before_seq: Option<i64>,
        limit: i64,
    ) -> AppResult<Vec<MessageView>> {
        let sql = format!(
            "{MESSAGE_VIEW_SELECT} WHERE m.conversation_id = $1 AND m.deleted_at IS NULL AND {SCOPE_COND} \
             AND ($3::bigint IS NULL OR m.seq < $3) ORDER BY m.seq DESC LIMIT $4"
        );
        let rows: Vec<MessageViewRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(conversation_id)
            .bind(scope.root())
            .bind(before_seq)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .db()?;
        let mut v = collect(rows, MessageViewRow::into_model)?;
        v.reverse();
        Ok(v)
    }

    /// Newest `limit` messages with seq > `after_seq` (ascending), plus how many such messages exist in total.
    pub async fn messages_after(
        &self,
        conversation_id: Uuid,
        scope: MessageScope,
        after_seq: i64,
        limit: i64,
    ) -> AppResult<(Vec<MessageView>, i64)> {
        let sql = format!(
            "{MESSAGE_VIEW_SELECT} WHERE m.conversation_id = $1 AND m.deleted_at IS NULL AND {SCOPE_COND} \
             AND m.seq > $3 ORDER BY m.seq DESC LIMIT $4"
        );
        let rows: Vec<MessageViewRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(conversation_id)
            .bind(scope.root())
            .bind(after_seq)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .db()?;
        let mut v = collect(rows, MessageViewRow::into_model)?;
        v.reverse();
        let count_sql = format!(
            "SELECT count(*) FROM messages m WHERE m.conversation_id = $1 AND m.deleted_at IS NULL \
             AND {SCOPE_COND} AND m.seq > $3"
        );
        let (total,): (i64,) = sqlx::query_as(AssertSqlSafe(count_sql))
            .bind(conversation_id)
            .bind(scope.root())
            .bind(after_seq)
            .fetch_one(&self.pool)
            .await
            .db()?;
        Ok((v, total))
    }

    pub async fn soft_delete_message(&self, id: Uuid) -> AppResult<bool> {
        let r = sqlx::query("UPDATE messages SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL")
            .bind(id)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn add_reaction(&self, message_id: Uuid, member_id: Uuid, emoji: &str) -> AppResult<bool> {
        let r = sqlx::query(
            "INSERT INTO reactions (message_id, member_id, emoji) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(message_id)
        .bind(member_id)
        .bind(emoji)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn remove_reaction(&self, message_id: Uuid, member_id: Uuid, emoji: &str) -> AppResult<bool> {
        let r = sqlx::query("DELETE FROM reactions WHERE message_id = $1 AND member_id = $2 AND emoji = $3")
            .bind(message_id)
            .bind(member_id)
            .bind(emoji)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn reaction_summary(&self, message_id: Uuid) -> AppResult<Vec<ReactionSummary>> {
        let (Json(v),): (Json<Vec<ReactionSummary>>,) = sqlx::query_as(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object('emoji', r.emoji, 'count', r.cnt, 'handles', r.handles) \
                    ORDER BY r.first_at), '[]'::jsonb) \
             FROM (SELECT rx.emoji, count(*) AS cnt, array_agg(rm.handle ORDER BY rx.created_at) AS handles, \
                          min(rx.created_at) AS first_at \
                   FROM reactions rx JOIN members rm ON rm.id = rx.member_id \
                   WHERE rx.message_id = $1 GROUP BY rx.emoji) r",
        )
        .bind(message_id)
        .fetch_one(&self.pool)
        .await
        .db()?;
        Ok(v)
    }

    /// Messages posted by an author outside any wake since `since`.
    pub async fn count_proactive_posts_since(&self, author_id: Uuid, since: DateTime<Utc>) -> AppResult<i64> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM messages WHERE author_id = $1 AND wake_id IS NULL AND created_at >= $2",
        )
        .bind(author_id)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .db()?;
        Ok(n)
    }

    /// The latest non-deleted message in the same scope before `seq` that was not written by `author_id`.
    pub async fn previous_other_message(
        &self,
        conversation_id: Uuid,
        scope: MessageScope,
        seq: i64,
        author_id: Uuid,
    ) -> AppResult<Option<Message>> {
        let sql = format!(
            "SELECT m.id, m.conversation_id, m.seq, m.thread_root_id, m.author_id, m.body, m.mentions, m.depth, \
             m.chain_root_id, m.wake_id, m.created_at, m.edited_at, m.deleted_at FROM messages m \
             WHERE m.conversation_id = $1 AND m.deleted_at IS NULL AND {SCOPE_COND} AND m.seq < $3 \
             AND m.author_id <> $4 ORDER BY m.seq DESC LIMIT 1"
        );
        let row: Option<MessageRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(conversation_id)
            .bind(scope.root())
            .bind(seq)
            .bind(author_id)
            .fetch_optional(&self.pool)
            .await
            .db()?;
        Ok(row.map(Into::into))
    }
}
