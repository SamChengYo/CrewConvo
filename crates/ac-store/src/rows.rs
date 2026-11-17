//! Raw row structs and their conversion into domain types.

use ac_core::AppResult;
use ac_core::model::*;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use sqlx::types::Json;
use uuid::Uuid;

pub(crate) const MEMBER_COLS: &str =
    "m.id, m.workspace_id, m.kind, m.handle, m.display_name, m.is_admin, m.created_at, m.deactivated_at";

#[derive(FromRow)]
pub(crate) struct MemberRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: String,
    pub handle: String,
    pub display_name: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub deactivated_at: Option<DateTime<Utc>>,
}

impl MemberRow {
    pub fn into_model(self) -> AppResult<Member> {
        Ok(Member {
            id: self.id,
            workspace_id: self.workspace_id,
            kind: self.kind.parse()?,
            handle: self.handle,
            display_name: self.display_name,
            is_admin: self.is_admin,
            created_at: self.created_at,
            deactivated_at: self.deactivated_at,
        })
    }
}

pub(crate) const AGENT_COLS: &str = "m.id, m.workspace_id, m.kind, m.handle, m.display_name, m.is_admin, \
     m.created_at, m.deactivated_at, a.origin, a.owner_id, a.description, a.expertise, a.status, a.model, \
     a.system_prompt, a.tools, a.sandbox_config, a.delivery, a.webhook_url, a.max_wake_seconds, \
     a.daily_token_budget, (a.token_hash IS NOT NULL) AS has_token, a.token_prefix, a.last_seen_at, \n     a.provider_connection_id, a.mention_only";

#[derive(FromRow)]
pub(crate) struct AgentRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: String,
    pub handle: String,
    pub display_name: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub deactivated_at: Option<DateTime<Utc>>,
    pub origin: String,
    pub owner_id: Uuid,
    pub description: String,
    pub expertise: Vec<String>,
    pub status: String,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub tools: Json<Value>,
    pub sandbox_config: Json<Value>,
    pub delivery: Option<String>,
    pub webhook_url: Option<String>,
    pub max_wake_seconds: i32,
    pub daily_token_budget: Option<i64>,
    pub has_token: bool,
    pub token_prefix: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub provider_connection_id: Option<Uuid>,
    pub mention_only: bool,
}

impl AgentRow {
    pub fn into_model(self) -> AppResult<Agent> {
        Ok(Agent {
            member: Member {
                id: self.id,
                workspace_id: self.workspace_id,
                kind: self.kind.parse()?,
                handle: self.handle,
                display_name: self.display_name,
                is_admin: self.is_admin,
                created_at: self.created_at,
                deactivated_at: self.deactivated_at,
            },
            origin: self.origin.parse()?,
            owner_id: self.owner_id,
            description: self.description,
            expertise: self.expertise,
            status: self.status.parse()?,
            model: self.model,
            system_prompt: self.system_prompt,
            tools: self.tools.0,
            sandbox_config: self.sandbox_config.0,
            delivery: self.delivery,
            webhook_url: self.webhook_url,
            max_wake_seconds: self.max_wake_seconds,
            daily_token_budget: self.daily_token_budget,
            has_token: self.has_token,
            token_prefix: self.token_prefix,
            last_seen_at: self.last_seen_at,
            provider_connection_id: self.provider_connection_id,
            mention_only: self.mention_only,
        })
    }
}

pub(crate) const CONV_COLS: &str = "c.id, c.workspace_id, c.kind, c.name, c.topic, c.visibility, \
     c.created_by, c.created_at, c.archived_at, c.frozen_until_human, c.next_seq";

#[derive(FromRow)]
pub(crate) struct ConversationRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub visibility: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub frozen_until_human: bool,
    pub next_seq: i64,
}

impl ConversationRow {
    pub fn into_model(self) -> AppResult<Conversation> {
        Ok(Conversation {
            id: self.id,
            workspace_id: self.workspace_id,
            kind: self.kind.parse()?,
            name: self.name,
            topic: self.topic,
            visibility: self.visibility.parse()?,
            created_by: self.created_by,
            created_at: self.created_at,
            archived_at: self.archived_at,
            frozen_until_human: self.frozen_until_human,
            next_seq: self.next_seq,
        })
    }
}

#[derive(FromRow)]
pub(crate) struct MessageRow {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub seq: i64,
    pub thread_root_id: Option<Uuid>,
    pub author_id: Uuid,
    pub body: String,
    pub mentions: Vec<Uuid>,
    pub depth: i32,
    pub chain_root_id: Uuid,
    pub wake_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl From<MessageRow> for Message {
    fn from(r: MessageRow) -> Self {
        Message {
            id: r.id,
            conversation_id: r.conversation_id,
            seq: r.seq,
            thread_root_id: r.thread_root_id,
            author_id: r.author_id,
            body: r.body,
            mentions: r.mentions,
            depth: r.depth,
            chain_root_id: r.chain_root_id,
            wake_id: r.wake_id,
            created_at: r.created_at,
            edited_at: r.edited_at,
            deleted_at: r.deleted_at,
        }
    }
}

/// Select list producing `MessageViewRow` from `messages m JOIN members a ON a.id = m.author_id`.
pub(crate) const MESSAGE_VIEW_SELECT: &str = r#"
SELECT m.id, m.conversation_id, m.seq, m.thread_root_id, m.author_id,
       a.handle AS author_handle, a.display_name AS author_display_name, a.kind AS author_kind,
       m.body,
       COALESCE((SELECT array_agg(mm.handle ORDER BY mm.handle) FROM members mm WHERE mm.id = ANY(m.mentions)),
                '{}'::text[]) AS mention_handles,
       COALESCE((SELECT jsonb_agg(jsonb_build_object('emoji', r.emoji, 'count', r.cnt, 'handles', r.handles)
                                  ORDER BY r.first_at)
                 FROM (SELECT rx.emoji, count(*) AS cnt, array_agg(rm.handle ORDER BY rx.created_at) AS handles,
                              min(rx.created_at) AS first_at
                       FROM reactions rx JOIN members rm ON rm.id = rx.member_id
                       WHERE rx.message_id = m.id GROUP BY rx.emoji) r),
                '[]'::jsonb) AS reactions,
       (SELECT count(*) FROM messages t WHERE t.thread_root_id = m.id AND t.deleted_at IS NULL) AS reply_count,
       m.depth, m.chain_root_id, m.wake_id, m.created_at, m.edited_at
FROM messages m JOIN members a ON a.id = m.author_id
"#;

#[derive(FromRow)]
pub(crate) struct MessageViewRow {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub seq: i64,
    pub thread_root_id: Option<Uuid>,
    pub author_id: Uuid,
    pub author_handle: String,
    pub author_display_name: String,
    pub author_kind: String,
    pub body: String,
    pub mention_handles: Vec<String>,
    pub reactions: Json<Vec<ReactionSummary>>,
    pub reply_count: i64,
    pub depth: i32,
    pub chain_root_id: Uuid,
    pub wake_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
}

impl MessageViewRow {
    pub fn into_model(self) -> AppResult<MessageView> {
        Ok(MessageView {
            id: self.id,
            conversation_id: self.conversation_id,
            seq: self.seq,
            thread_root_id: self.thread_root_id,
            author_id: self.author_id,
            author_handle: self.author_handle,
            author_display_name: self.author_display_name,
            author_kind: self.author_kind.parse()?,
            body: self.body,
            mentions: self.mention_handles,
            reactions: self.reactions.0,
            reply_count: self.reply_count,
            depth: self.depth,
            chain_root_id: self.chain_root_id,
            wake_id: self.wake_id,
            created_at: self.created_at,
            edited_at: self.edited_at,
        })
    }
}

#[derive(FromRow)]
pub(crate) struct WakeRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub conversation_id: Uuid,
    pub thread_root_id: Option<Uuid>,
    pub trigger_message_id: Uuid,
    pub reason: String,
    pub mode: String,
    pub discussion_id: Option<Uuid>,
    pub status: String,
    pub depth: i32,
    pub chain_root_id: Uuid,
    pub posted_count: i32,
    pub tokens_used: i64,
    pub private_log: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl WakeRow {
    pub fn into_model(self) -> AppResult<Wake> {
        Ok(Wake {
            id: self.id,
            agent_id: self.agent_id,
            conversation_id: self.conversation_id,
            thread_root_id: self.thread_root_id,
            trigger_message_id: self.trigger_message_id,
            reason: self.reason.parse()?,
            mode: self.mode.parse()?,
            discussion_id: self.discussion_id,
            status: self.status.parse()?,
            depth: self.depth,
            chain_root_id: self.chain_root_id,
            posted_count: self.posted_count,
            tokens_used: self.tokens_used,
            private_log: self.private_log,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
        })
    }
}

pub(crate) fn collect<T, R>(rows: Vec<R>, f: impl Fn(R) -> AppResult<T>) -> AppResult<Vec<T>> {
    rows.into_iter().map(f).collect()
}
