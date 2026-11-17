use ac_core::model::{Wake, WakeMode, WakeReason, WakeStatus};
use ac_core::{AppResult, new_id};
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use crate::rows::{WakeRow, collect};
use crate::{DbResultExt, Store};

const WAKE_COLS: &str = "id, agent_id, conversation_id, thread_root_id, trigger_message_id, reason, mode, \
     discussion_id, status, depth, chain_root_id, posted_count, tokens_used, private_log, created_at, started_at, \
     finished_at";

#[derive(Debug, Clone)]
pub struct NewWake {
    pub agent_id: Uuid,
    pub conversation_id: Uuid,
    pub thread_root_id: Option<Uuid>,
    pub trigger_message_id: Uuid,
    pub reason: WakeReason,
    pub mode: WakeMode,
    pub discussion_id: Option<Uuid>,
    pub depth: i32,
    pub chain_root_id: Uuid,
}

impl Store {
    /// Creates a pending wake, or merges into an existing pending wake for the same
    /// (agent, conversation, thread) by moving its trigger forward (spec 6.3).
    /// Returns the wake and whether it was merged.
    pub async fn create_or_merge_wake(&self, w: &NewWake) -> AppResult<(Wake, bool)> {
        let mut tx = self.pool.begin().await.db()?;
        let merge_sql = format!(
            "UPDATE wakes SET trigger_message_id = $4, depth = $5, chain_root_id = $6 \
             WHERE id = (SELECT id FROM wakes WHERE agent_id = $1 AND conversation_id = $2 \
                         AND thread_root_id IS NOT DISTINCT FROM $3 AND status = 'pending' \
                         AND discussion_id IS NULL ORDER BY created_at LIMIT 1 FOR UPDATE) \
             RETURNING {WAKE_COLS}"
        );
        let merged: Option<WakeRow> = if w.discussion_id.is_none() {
            sqlx::query_as(AssertSqlSafe(merge_sql))
                .bind(w.agent_id)
                .bind(w.conversation_id)
                .bind(w.thread_root_id)
                .bind(w.trigger_message_id)
                .bind(w.depth)
                .bind(w.chain_root_id)
                .fetch_optional(&mut *tx)
                .await
                .db()?
        } else {
            None
        };
        if let Some(row) = merged {
            tx.commit().await.db()?;
            return Ok((row.into_model()?, true));
        }
        let insert_sql = format!(
            "INSERT INTO wakes (id, agent_id, conversation_id, thread_root_id, trigger_message_id, reason, mode, \
             discussion_id, depth, chain_root_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING {WAKE_COLS}"
        );
        let row: WakeRow = sqlx::query_as(AssertSqlSafe(insert_sql))
            .bind(new_id())
            .bind(w.agent_id)
            .bind(w.conversation_id)
            .bind(w.thread_root_id)
            .bind(w.trigger_message_id)
            .bind(w.reason.as_str())
            .bind(w.mode.as_str())
            .bind(w.discussion_id)
            .bind(w.depth)
            .bind(w.chain_root_id)
            .fetch_one(&mut *tx)
            .await
            .db()?;
        tx.commit().await.db()?;
        Ok((row.into_model()?, false))
    }

    pub async fn get_wake(&self, id: Uuid) -> AppResult<Option<Wake>> {
        let sql = format!("SELECT {WAKE_COLS} FROM wakes WHERE id = $1");
        let row: Option<WakeRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()?;
        row.map(WakeRow::into_model).transpose()
    }

    pub async fn next_pending_wake(&self, agent_id: Uuid) -> AppResult<Option<Wake>> {
        let sql = format!(
            "SELECT {WAKE_COLS} FROM wakes WHERE agent_id = $1 AND status = 'pending' ORDER BY created_at LIMIT 1"
        );
        let row: Option<WakeRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(agent_id).fetch_optional(&self.pool).await.db()?;
        row.map(WakeRow::into_model).transpose()
    }

    /// Agents that currently have pending wakes.
    pub async fn agents_with_pending_wakes(&self) -> AppResult<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT DISTINCT agent_id FROM wakes WHERE status = 'pending'")
            .fetch_all(&self.pool)
            .await
            .db()?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Conditional status transition. Returns the updated wake if the current status was in `from`.
    pub async fn transition_wake(&self, id: Uuid, from: &[WakeStatus], to: WakeStatus) -> AppResult<Option<Wake>> {
        let from: Vec<&str> = from.iter().map(|s| s.as_str()).collect();
        let sql = format!(
            "UPDATE wakes SET status = $3, \
               started_at = CASE WHEN $3 IN ('delivered','running') THEN COALESCE(started_at, now()) ELSE started_at END, \
               finished_at = CASE WHEN $3 IN ('done','timeout','failed','cancelled') THEN now() ELSE finished_at END \
             WHERE id = $1 AND status = ANY($2) RETURNING {WAKE_COLS}"
        );
        let row: Option<WakeRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(&from)
            .bind(to.as_str())
            .fetch_optional(&self.pool)
            .await
            .db()?;
        row.map(WakeRow::into_model).transpose()
    }

    /// Finishes an in-flight wake with the given terminal status; no-op if it is already terminal.
    pub async fn finish_wake(
        &self,
        id: Uuid,
        status: WakeStatus,
        private_log: Option<&str>,
        tokens_used: Option<i64>,
    ) -> AppResult<Option<Wake>> {
        let sql = format!(
            "UPDATE wakes SET status = $2, finished_at = now(), \
               private_log = COALESCE($3, private_log), tokens_used = tokens_used + COALESCE($4, 0) \
             WHERE id = $1 AND status IN ('pending','delivered','running') RETURNING {WAKE_COLS}"
        );
        let row: Option<WakeRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(id)
            .bind(status.as_str())
            .bind(private_log)
            .bind(tokens_used)
            .fetch_optional(&self.pool)
            .await
            .db()?;
        row.map(WakeRow::into_model).transpose()
    }

    /// Adds tokens to a wake regardless of status (late reports after cancellation still count).
    pub async fn add_wake_tokens(&self, id: Uuid, tokens: i64) -> AppResult<()> {
        sqlx::query("UPDATE wakes SET tokens_used = tokens_used + $2 WHERE id = $1")
            .bind(id)
            .bind(tokens)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }

    pub async fn increment_posted(&self, id: Uuid) -> AppResult<()> {
        sqlx::query("UPDATE wakes SET posted_count = posted_count + 1 WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }

    /// Cancels all non-terminal wakes of a conversation, returning them (with their previous status).
    pub async fn cancel_conversation_wakes(&self, conversation_id: Uuid) -> AppResult<Vec<(Wake, WakeStatus)>> {
        self.cancel_where("conversation_id = $1 AND status IN ('pending','delivered','running')", conversation_id).await
    }

    /// Cancels the pending (not yet started) wakes of a chain.
    pub async fn cancel_pending_chain_wakes(&self, chain_root_id: Uuid) -> AppResult<Vec<(Wake, WakeStatus)>> {
        self.cancel_where("chain_root_id = $1 AND status = 'pending'", chain_root_id).await
    }

    async fn cancel_where(&self, cond: &str, id: Uuid) -> AppResult<Vec<(Wake, WakeStatus)>> {
        let cols = WAKE_COLS.split(',').map(|c| format!("w.{}", c.trim())).collect::<Vec<_>>().join(", ");
        let sql = format!(
            "WITH old AS (SELECT id, status AS old_status FROM wakes WHERE {cond} FOR UPDATE) \
             UPDATE wakes w SET status = 'cancelled', finished_at = now() FROM old WHERE w.id = old.id \
             RETURNING {cols}, old.old_status"
        );
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            w: WakeRow,
            old_status: String,
        }
        let rows: Vec<Row> = sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_all(&self.pool).await.db()?;
        collect(rows, |r| Ok((r.w.into_model()?, r.old_status.parse()?)))
    }

    /// (number of wakes, total tokens) of a chain.
    pub async fn chain_stats(&self, chain_root_id: Uuid) -> AppResult<(i64, i64)> {
        let (n, t): (i64, i64) = sqlx::query_as(
            "SELECT count(*), COALESCE(sum(tokens_used), 0)::bigint FROM wakes WHERE chain_root_id = $1",
        )
        .bind(chain_root_id)
        .fetch_one(&self.pool)
        .await
        .db()?;
        Ok((n, t))
    }

    pub async fn agent_tokens_since(&self, agent_id: Uuid, since: DateTime<Utc>) -> AppResult<i64> {
        let (t,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(sum(tokens_used), 0)::bigint FROM wakes WHERE agent_id = $1 AND created_at >= $2",
        )
        .bind(agent_id)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .db()?;
        Ok(t)
    }

    pub async fn recent_private_logs(
        &self,
        agent_id: Uuid,
        conversation_id: Uuid,
        limit: i64,
    ) -> AppResult<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT private_log FROM wakes WHERE agent_id = $1 AND conversation_id = $2 \
             AND private_log IS NOT NULL AND private_log <> '' ORDER BY finished_at DESC NULLS LAST LIMIT $3",
        )
        .bind(agent_id)
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .db()?;
        let mut v: Vec<String> = rows.into_iter().map(|r| r.0).collect();
        v.reverse();
        Ok(v)
    }

    pub async fn wakes_for_trigger(&self, message_id: Uuid) -> AppResult<Vec<Wake>> {
        let sql = format!("SELECT {WAKE_COLS} FROM wakes WHERE trigger_message_id = $1 ORDER BY created_at");
        let rows: Vec<WakeRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(message_id).fetch_all(&self.pool).await.db()?;
        collect(rows, WakeRow::into_model)
    }

    pub async fn active_wakes_for_conversation(&self, conversation_id: Uuid) -> AppResult<Vec<Wake>> {
        let sql = format!(
            "SELECT {WAKE_COLS} FROM wakes WHERE conversation_id = $1 \
             AND status IN ('pending','delivered','running') ORDER BY created_at"
        );
        let rows: Vec<WakeRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(conversation_id).fetch_all(&self.pool).await.db()?;
        collect(rows, WakeRow::into_model)
    }

    pub async fn wakes_for_chain(&self, chain_root_id: Uuid) -> AppResult<Vec<Wake>> {
        let sql = format!("SELECT {WAKE_COLS} FROM wakes WHERE chain_root_id = $1 ORDER BY created_at");
        let rows: Vec<WakeRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(chain_root_id).fetch_all(&self.pool).await.db()?;
        collect(rows, WakeRow::into_model)
    }

    /// On startup: wakes left in flight by a previous process can never complete.
    pub async fn fail_orphaned_wakes(&self) -> AppResult<u64> {
        let r = sqlx::query(
            "UPDATE wakes SET status = 'failed', finished_at = now() WHERE status IN ('delivered','running')",
        )
        .execute(&self.pool)
        .await
        .db()?;
        Ok(r.rows_affected())
    }
}

impl Store {
    /// The wake an agent is currently working on (at most one per agent).
    pub async fn inflight_wake_for_agent(&self, agent_id: Uuid) -> AppResult<Option<Wake>> {
        let sql = format!(
            "SELECT {WAKE_COLS} FROM wakes WHERE agent_id = $1 AND status IN ('delivered','running') \
             ORDER BY started_at DESC NULLS LAST LIMIT 1"
        );
        let row: Option<WakeRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(agent_id).fetch_optional(&self.pool).await.db()?;
        row.map(WakeRow::into_model).transpose()
    }

    pub async fn wakes_for_discussion(&self, discussion_id: Uuid) -> AppResult<Vec<Wake>> {
        let sql = format!("SELECT {WAKE_COLS} FROM wakes WHERE discussion_id = $1 ORDER BY created_at");
        let rows: Vec<WakeRow> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(discussion_id).fetch_all(&self.pool).await.db()?;
        collect(rows, WakeRow::into_model)
    }

    /// Cancels the not-yet-finished wakes of a discussion.
    pub async fn cancel_discussion_wakes(&self, discussion_id: Uuid) -> AppResult<Vec<(Wake, WakeStatus)>> {
        self.cancel_where("discussion_id = $1 AND status IN ('pending','delivered','running')", discussion_id).await
    }

    /// Wakes produced by a message: the wake whose trigger is the message, plus the wake that
    /// produced the message itself (for the "why" view).
    pub async fn wake_by_id_opt(&self, id: Option<Uuid>) -> AppResult<Option<Wake>> {
        match id {
            Some(id) => self.get_wake(id).await,
            None => Ok(None),
        }
    }
}

/// One row of the workspace activity feed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WakeActivity {
    #[serde(flatten)]
    pub wake: Wake,
    pub agent_handle: String,
    pub agent_display_name: String,
    pub conversation_kind: String,
    pub conversation_name: Option<String>,
    pub trigger_excerpt: Option<String>,
    pub trigger_author_handle: Option<String>,
}

impl Store {
    pub async fn set_wake_payload(&self, id: Uuid, payload: &serde_json::Value) -> AppResult<()> {
        sqlx::query("UPDATE wakes SET payload = $2 WHERE id = $1")
            .bind(id)
            .bind(sqlx::types::Json(payload))
            .execute(&self.pool)
            .await
            .db()?;
        Ok(())
    }

    pub async fn wake_payload(&self, id: Uuid) -> AppResult<Option<serde_json::Value>> {
        let row: Option<(Option<sqlx::types::Json<serde_json::Value>>,)> =
            sqlx::query_as("SELECT payload FROM wakes WHERE id = $1").bind(id).fetch_optional(&self.pool).await.db()?;
        Ok(row.and_then(|r| r.0).map(|j| j.0))
    }

    /// Recent wakes in conversations the member belongs to (newest first).
    pub async fn wake_activity(
        &self,
        member_id: Uuid,
        before: Option<chrono::DateTime<chrono::Utc>>,
        limit: i64,
    ) -> AppResult<Vec<WakeActivity>> {
        let cols = WAKE_COLS.split(',').map(|c| format!("w.{}", c.trim())).collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT {cols}, a.handle AS agent_handle, a.display_name AS agent_display_name, c.kind AS conversation_kind, \
                    c.name AS conversation_name, left(t.body, 140) AS trigger_excerpt, ta.handle AS trigger_author_handle \
             FROM wakes w \
             JOIN members a ON a.id = w.agent_id \
             JOIN conversations c ON c.id = w.conversation_id \
             JOIN conversation_members cm ON cm.conversation_id = c.id AND cm.member_id = $1 \
             LEFT JOIN messages t ON t.id = w.trigger_message_id \
             LEFT JOIN members ta ON ta.id = t.author_id \
             WHERE ($2::timestamptz IS NULL OR w.created_at < $2) \
             ORDER BY w.created_at DESC LIMIT $3"
        );
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            w: WakeRow,
            agent_handle: String,
            agent_display_name: String,
            conversation_kind: String,
            conversation_name: Option<String>,
            trigger_excerpt: Option<String>,
            trigger_author_handle: Option<String>,
        }
        let rows: Vec<Row> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(member_id)
            .bind(before)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .db()?;
        collect(rows, |r| {
            Ok(WakeActivity {
                wake: r.w.into_model()?,
                agent_handle: r.agent_handle,
                agent_display_name: r.agent_display_name,
                conversation_kind: r.conversation_kind,
                conversation_name: r.conversation_name,
                trigger_excerpt: r.trigger_excerpt,
                trigger_author_handle: r.trigger_author_handle,
            })
        })
    }
}
