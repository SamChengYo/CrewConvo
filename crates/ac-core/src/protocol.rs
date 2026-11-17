//! Agent protocol (spec 5.1, 7.3). Builtin and external agents speak exactly this.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{ConversationKind, MemberKind, MessageView, ReactionSummary, WakeMode, WakeReason};

/// Platform → agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    #[serde(rename = "wake")]
    Wake { payload: Box<WakePayload> },
    #[serde(rename = "wake.cancelled")]
    WakeCancelled { wake_id: Uuid },
    #[serde(rename = "ping")]
    Ping,
}

/// Agent → platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentReply {
    #[serde(rename = "wake.ack")]
    WakeAck { wake_id: Uuid },
    #[serde(rename = "wake.heartbeat")]
    WakeHeartbeat { wake_id: Uuid },
    #[serde(rename = "wake.done")]
    WakeDone {
        wake_id: Uuid,
        #[serde(default)]
        private_log: Option<String>,
        #[serde(default)]
        tokens_used: Option<i64>,
    },
}

impl AgentReply {
    pub fn wake_id(&self) -> Uuid {
        match self {
            AgentReply::WakeAck { wake_id }
            | AgentReply::WakeHeartbeat { wake_id }
            | AgentReply::WakeDone { wake_id, .. } => *wake_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadMember {
    pub handle: String,
    pub kind: MemberKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadConversation {
    pub id: Uuid,
    pub kind: ConversationKind,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub members: Vec<PayloadMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadMessage {
    pub id: Uuid,
    pub seq: i64,
    pub author_handle: String,
    pub author_kind: MemberKind,
    pub body: String,
    pub thread_root_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub mentions: Vec<String>,
    pub reactions: Vec<ReactionSummary>,
}

impl From<&MessageView> for PayloadMessage {
    fn from(m: &MessageView) -> Self {
        PayloadMessage {
            id: m.id,
            seq: m.seq,
            author_handle: m.author_handle.clone(),
            author_kind: m.author_kind,
            body: m.body.clone(),
            thread_root_id: m.thread_root_id,
            created_at: m.created_at,
            mentions: m.mentions.clone(),
            reactions: m.reactions.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadLimits {
    pub depth: i32,
    pub max_depth: i32,
    pub deadline_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadDiscussion {
    pub id: Uuid,
    /// `turn` (your turn to speak or skip) or `conclusion` (write the conclusion).
    pub phase: String,
    /// 1-based round number.
    pub round: i32,
    pub max_rounds: i32,
    pub participants: Vec<String>,
    pub instruction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakePayload {
    pub wake_id: Uuid,
    pub reason: WakeReason,
    pub mode: WakeMode,
    pub conversation: PayloadConversation,
    pub thread_root_id: Option<Uuid>,
    /// Present when woken inside a thread.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_root: Option<PayloadMessage>,
    pub trigger_message_id: Uuid,
    pub summary: Option<String>,
    pub recent: Vec<PayloadMessage>,
    pub unread: Vec<PayloadMessage>,
    pub unread_omitted: i64,
    pub previous_private_logs: Vec<String>,
    pub discussion: Option<PayloadDiscussion>,
    pub also_woken: Vec<String>,
    pub limits: PayloadLimits,
}

impl WakePayload {
    /// Highest seq contained in the payload (used to advance the cursor on ack).
    pub fn max_seq(&self) -> i64 {
        self.recent.iter().chain(self.unread.iter()).map(|m| m.seq).max().unwrap_or(0)
    }
}
