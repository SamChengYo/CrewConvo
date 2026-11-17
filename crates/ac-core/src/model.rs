use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::str_enum;

str_enum! {
    pub enum MemberKind { Human => "human", Agent => "agent", System => "system" }
}
str_enum! {
    pub enum AgentOrigin { Builtin => "builtin", External => "external" }
}
str_enum! {
    pub enum AgentStatus { Online => "online", Offline => "offline", Busy => "busy", Disabled => "disabled" }
}
str_enum! {
    pub enum ConversationKind { Channel => "channel", Dm => "dm" }
}
str_enum! {
    pub enum Visibility { Public => "public", Private => "private" }
}
str_enum! {
    pub enum WakeReason {
        Mention => "mention",
        Dm => "dm",
        ThreadReply => "thread_reply",
        Sticky => "sticky",
        Decider => "decider",
        DiscussionTurn => "discussion_turn",
    }
}
str_enum! {
    pub enum WakeMode { Single => "single", Parallel => "parallel", Discussion => "discussion" }
}
str_enum! {
    pub enum WakeStatus {
        Pending => "pending",
        Delivered => "delivered",
        Running => "running",
        Done => "done",
        Timeout => "timeout",
        Failed => "failed",
        Cancelled => "cancelled",
    }
}

impl WakeStatus {
    /// The wake has been handed to the agent and has not finished.
    pub fn is_in_flight(&self) -> bool {
        matches!(self, WakeStatus::Delivered | WakeStatus::Running)
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, WakeStatus::Done | WakeStatus::Timeout | WakeStatus::Failed | WakeStatus::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: MemberKind,
    pub handle: String,
    pub display_name: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub deactivated_at: Option<DateTime<Utc>>,
}

/// Full agent record. Secrets (token hash, webhook secret) are never part of this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub member: Member,
    pub origin: AgentOrigin,
    pub owner_id: Uuid,
    pub description: String,
    pub expertise: Vec<String>,
    pub status: AgentStatus,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub tools: Value,
    pub sandbox_config: Value,
    pub delivery: Option<String>,
    pub webhook_url: Option<String>,
    pub max_wake_seconds: i32,
    pub daily_token_budget: Option<i64>,
    pub has_token: bool,
    pub token_prefix: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Set for agents synced from a platform connection (woken over A2A).
    #[serde(default)]
    pub provider_connection_id: Option<Uuid>,
    /// Never offered to the decision model.
    #[serde(default)]
    pub mention_only: bool,
}

/// What other agents and the Decider see about an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub origin: AgentOrigin,
    pub description: String,
    pub expertise: Vec<String>,
    pub status: AgentStatus,
}

impl From<&Agent> for AgentProfile {
    fn from(a: &Agent) -> Self {
        AgentProfile {
            id: a.member.id,
            handle: a.member.handle.clone(),
            display_name: a.member.display_name.clone(),
            origin: a.origin,
            description: a.description.clone(),
            expertise: a.expertise.clone(),
            status: a.status,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: ConversationKind,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub visibility: Visibility,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub frozen_until_human: bool,
    pub next_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMemberView {
    pub member_id: Uuid,
    pub handle: String,
    pub display_name: String,
    pub kind: MemberKind,
    pub description: Option<String>,
    pub agent_status: Option<AgentStatus>,
    pub last_read_seq: i64,
}

/// Stored message row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReactionSummary {
    pub emoji: String,
    pub count: i64,
    pub handles: Vec<String>,
}

/// Message enriched for display / agent consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageView {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub seq: i64,
    pub thread_root_id: Option<Uuid>,
    pub author_id: Uuid,
    pub author_handle: String,
    pub author_display_name: String,
    pub author_kind: MemberKind,
    pub body: String,
    pub mentions: Vec<String>,
    pub reactions: Vec<ReactionSummary>,
    pub reply_count: i64,
    pub depth: i32,
    pub chain_root_id: Uuid,
    pub wake_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wake {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub conversation_id: Uuid,
    pub thread_root_id: Option<Uuid>,
    pub trigger_message_id: Uuid,
    pub reason: WakeReason,
    pub mode: WakeMode,
    pub discussion_id: Option<Uuid>,
    pub status: WakeStatus,
    pub depth: i32,
    pub chain_root_id: Uuid,
    pub posted_count: i32,
    pub tokens_used: i64,
    pub private_log: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

str_enum! {
    pub enum DeciderProtocol { DecisionsApi => "decisions_api", LlmStructured => "llm_structured" }
}
str_enum! {
    pub enum Interaction { Independent => "independent", Discuss => "discuss" }
}
str_enum! {
    pub enum DiscussionStatus { Running => "running", Summarizing => "summarizing", Done => "done", Cancelled => "cancelled" }
}

/// Decider configuration as exposed by the API. The API key itself never leaves the server:
/// only whether one is set and its last 4 characters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeciderConfig {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub protocol: DeciderProtocol,
    pub endpoint_url: String,
    pub has_api_key: bool,
    pub api_key_last4: Option<String>,
    pub auth_header: String,
    pub auth_prefix: String,
    pub model: String,
    pub input_field: String,
    pub timeout_ms: i32,
    pub t_wake: f32,
    pub t_discuss: f32,
    pub max_wake: i32,
    pub extra_headers: Value,
    pub extra_body: Value,
    pub enabled: bool,
    pub priority: i32,
    pub updated_by: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub id: Uuid,
    pub trigger_message_id: Uuid,
    pub decider_config_id: Option<Uuid>,
    /// `rule` (rule layer), `decider`, or `none` (nothing applied).
    pub source: String,
    pub reason: Option<String>,
    pub attempts: Value,
    pub request: Option<Value>,
    pub response: Option<Value>,
    pub scores: Option<Value>,
    pub interaction: Option<String>,
    pub interaction_confidence: Option<f32>,
    pub woken: Vec<Uuid>,
    pub mode: Option<String>,
    pub latency_ms: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discussion {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub thread_root_id: Uuid,
    pub participants: Vec<Uuid>,
    pub round: i32,
    pub max_rounds: i32,
    pub status: DiscussionStatus,
    pub summarizer_id: Option<Uuid>,
    pub trigger_message_id: Option<Uuid>,
    pub chain_root_id: Option<Uuid>,
    pub depth: i32,
    pub turn_index: i32,
    pub spoke_this_round: bool,
    pub human_interjected: bool,
    pub end_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub thread_root_id: Option<Uuid>,
    pub covers_up_to_seq: i64,
    pub body: String,
    pub token_count: i32,
    pub created_at: DateTime<Utc>,
}
