//! Discussion state (spec 6.5).

use ac_core::model::{Discussion, DiscussionStatus};
use ac_core::{AppResult, new_id};
use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use crate::rows::collect;
use crate::{DbResultExt, Store};

const COLS: &str = "id, conversation_id, thread_root_id, participants, round, max_rounds, status, summarizer_id, \
     trigger_message_id, chain_root_id, depth, turn_index, spoke_this_round, human_interjected, end_reason, \
     created_at, finished_at";

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    conversation_id: Uuid,
    thread_root_id: Uuid,
    participants: Vec<Uuid>,
    round: i32,
    max_rounds: i32,
    status: String,
    summarizer_id: Option<Uuid>,
    trigger_message_id: Option<Uuid>,
    chain_root_id: Option<Uuid>,
    depth: i32,
    turn_index: i32,
    spoke_this_round: bool,
    human_interjected: bool,
    end_reason: Option<String>,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

impl Row {
    fn into_model(self) -> AppResult<Discussion> {
        Ok(Discussion {
            id: self.id,
            conversation_id: self.conversation_id,
            thread_root_id: self.thread_root_id,
            participants: self.participants,
            round: self.round,
            max_rounds: self.max_rounds,
            status: self.status.parse()?,
            summarizer_id: self.summarizer_id,
            trigger_message_id: self.trigger_message_id,
            chain_root_id: self.chain_root_id,
            depth: self.depth,
            turn_index: self.turn_index,
            spoke_this_round: self.spoke_this_round,
            human_interjected: self.human_interjected,
            end_reason: self.end_reason,
            created_at: self.created_at,
            finished_at: self.finished_at,
        })
    }
}

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub async fn create_discussion(
        &self,
        conversation_id: Uuid,
        thread_root_id: Uuid,
        participants: &[Uuid],
        max_rounds: i32,
        summarizer_id: Uuid,
        trigger_message_id: Uuid,
        chain_root_id: Uuid,
        depth: i32,
    ) -> AppResult<Discussion> {
        let sql = format!(
            "INSERT INTO discussions (id, conversation_id, thread_root_id, participants, max_rounds, summarizer_id, \
             trigger_message_id, chain_root_id, depth) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING {COLS}"
        );
        let row: Row = sqlx::query_as(AssertSqlSafe(sql))
            .bind(new_id())
            .bind(conversation_id)
            .bind(thread_root_id)
            .bind(participants)
            .bind(max_rounds)
            .bind(summarizer_id)
            .bind(trigger_message_id)
            .bind(chain_root_id)
            .bind(depth)
            .fetch_one(&self.pool)
            .await
            .db()?;
        row.into_model()
    }

    pub async fn get_discussion(&self, id: Uuid) -> AppResult<Option<Discussion>> {
        let sql = format!("SELECT {COLS} FROM discussions WHERE id = $1");
        let row: Option<Row> = sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await.db()?;
        row.map(Row::into_model).transpose()
    }

    /// The running (or summarizing) discussion of a thread, if any.
    pub async fn active_discussion(
        &self,
        conversation_id: Uuid,
        thread_root_id: Uuid,
    ) -> AppResult<Option<Discussion>> {
        let sql = format!(
            "SELECT {COLS} FROM discussions WHERE conversation_id = $1 AND thread_root_id = $2 \
             AND status IN ('running','summarizing') ORDER BY created_at DESC LIMIT 1"
        );
        let row: Option<Row> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(conversation_id)
            .bind(thread_root_id)
            .fetch_optional(&self.pool)
            .await
            .db()?;
        row.map(Row::into_model).transpose()
    }

    pub async fn active_discussions_in_conversation(&self, conversation_id: Uuid) -> AppResult<Vec<Discussion>> {
        let sql = format!(
            "SELECT {COLS} FROM discussions WHERE conversation_id = $1 AND status IN ('running','summarizing') \
             ORDER BY created_at"
        );
        let rows: Vec<Row> =
            sqlx::query_as(AssertSqlSafe(sql)).bind(conversation_id).fetch_all(&self.pool).await.db()?;
        collect(rows, Row::into_model)
    }

    /// Persists the scheduling state of a discussion.
    pub async fn save_discussion(&self, d: &Discussion) -> AppResult<()> {
        sqlx::query(
            "UPDATE discussions SET participants = $2, round = $3, status = $4, turn_index = $5, \
             spoke_this_round = $6, human_interjected = $7, end_reason = $8, \
             finished_at = CASE WHEN $4 IN ('done','cancelled') THEN COALESCE(finished_at, now()) ELSE NULL END, \
             updated_at = now() WHERE id = $1",
        )
        .bind(d.id)
        .bind(&d.participants)
        .bind(d.round)
        .bind(d.status.as_str())
        .bind(d.turn_index)
        .bind(d.spoke_this_round)
        .bind(d.human_interjected)
        .bind(&d.end_reason)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }

    pub async fn set_discussion_status(&self, id: Uuid, status: DiscussionStatus, reason: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE discussions SET status = $2, end_reason = COALESCE(end_reason, $3), \
             finished_at = CASE WHEN $2 IN ('done','cancelled') THEN now() ELSE finished_at END, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(status.as_str())
        .bind(reason)
        .execute(&self.pool)
        .await
        .db()?;
        Ok(())
    }
}
