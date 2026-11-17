//! In-process realtime event bus (spec 3: tokio broadcast is enough for one node).

use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::model::{Conversation, MessageView, ReactionSummary, WakeMode, WakeReason, WakeStatus};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RealtimeEvent {
    MessageCreated {
        message: MessageView,
    },
    MessageDeleted {
        conversation_id: Uuid,
        message_id: Uuid,
    },
    ReactionsChanged {
        conversation_id: Uuid,
        message_id: Uuid,
        reactions: Vec<ReactionSummary>,
    },
    WakeStatus {
        conversation_id: Uuid,
        wake_id: Uuid,
        agent_id: Uuid,
        agent_handle: String,
        thread_root_id: Option<Uuid>,
        reason: WakeReason,
        mode: WakeMode,
        status: WakeStatus,
    },
    ConversationUpdated {
        conversation: Conversation,
    },
    MembershipChanged {
        conversation_id: Uuid,
        member_id: Uuid,
        joined: bool,
    },
    AgentsChanged {
        workspace_id: Uuid,
    },
}

impl RealtimeEvent {
    /// Conversation the event belongs to (None = workspace wide).
    pub fn conversation_id(&self) -> Option<Uuid> {
        match self {
            RealtimeEvent::MessageCreated { message } => Some(message.conversation_id),
            RealtimeEvent::MessageDeleted { conversation_id, .. }
            | RealtimeEvent::ReactionsChanged { conversation_id, .. }
            | RealtimeEvent::WakeStatus { conversation_id, .. }
            | RealtimeEvent::MembershipChanged { conversation_id, .. } => Some(*conversation_id),
            RealtimeEvent::ConversationUpdated { conversation } => Some(conversation.id),
            RealtimeEvent::AgentsChanged { .. } => None,
        }
    }
}

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<RealtimeEvent>,
}

impl Bus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self { tx }
    }
    pub fn publish(&self, ev: RealtimeEvent) {
        let _ = self.tx.send(ev);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RealtimeEvent> {
        self.tx.subscribe()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}
