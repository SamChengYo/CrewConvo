//! Agent connections. Builtin runtime and external transports (WebSocket / webhook)
//! all register here and speak the same event/reply protocol.

use std::sync::atomic::Ordering;

use ac_core::protocol::{AgentEvent, AgentReply};
use ac_core::{AgentId, AppError, AppResult};
use tokio::sync::mpsc;

use crate::{WakeEngine, WakeSignal};

pub(crate) struct Conn {
    pub id: usize,
    pub events: mpsc::UnboundedSender<AgentEvent>,
}

/// A live connection of one agent: receive events, send replies.
pub struct AgentLink {
    pub agent_id: AgentId,
    pub events: mpsc::UnboundedReceiver<AgentEvent>,
    pub replier: AgentReplier,
}

/// Cloneable reply handle of a connection.
#[derive(Clone)]
pub struct AgentReplier {
    agent_id: AgentId,
    conn_id: usize,
    engine: WakeEngine,
}

impl WakeEngine {
    /// Registers a connection for an agent, replacing any previous one.
    pub fn connect(&self, agent_id: AgentId) -> AgentLink {
        let (tx, rx) = mpsc::unbounded_channel();
        let id = self.inner.next_conn_id.fetch_add(1, Ordering::SeqCst);
        self.inner.conns.lock().unwrap().insert(agent_id, Conn { id, events: tx });
        let engine = self.clone();
        // A newly connected agent may have queued wakes.
        let e = self.clone();
        tokio::spawn(async move { e.ensure_worker(agent_id).await });
        AgentLink { agent_id, events: rx, replier: AgentReplier { agent_id, conn_id: id, engine } }
    }
}

impl AgentReplier {
    pub fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    /// Routes a reply to the worker handling the wake.
    pub async fn reply(&self, reply: AgentReply) -> AppResult<()> {
        let wake_id = reply.wake_id();
        if let AgentReply::WakeDone { tokens_used: Some(t), .. } = &reply {
            // Tokens reported after a cancellation still count toward budgets.
            if !self.engine.inner.inflight.lock().unwrap().contains_key(&wake_id) {
                let wake = self.engine.inner.store.get_wake(wake_id).await?;
                if let Some(w) = wake
                    && w.agent_id == self.agent_id
                    && *t > 0
                {
                    self.engine.inner.store.add_wake_tokens(wake_id, *t).await?;
                }
                return Ok(());
            }
        }
        let wake = self.engine.inner.store.get_wake(wake_id).await?.ok_or_else(|| AppError::not_found("wake"))?;
        if wake.agent_id != self.agent_id {
            return Err(AppError::forbidden("wake 不屬於此 Agent"));
        }
        self.engine.signal(wake_id, WakeSignal::Reply(reply));
        Ok(())
    }

    /// Removes the connection if it is still the current one.
    pub fn disconnect(&self) {
        let mut conns = self.engine.inner.conns.lock().unwrap();
        if conns.get(&self.agent_id).map(|c| c.id) == Some(self.conn_id) {
            conns.remove(&self.agent_id);
        }
    }
}

impl WakeEngine {
    /// Drops an agent's live connection (token rotated / revoked / agent deleted).
    pub fn disconnect_agent(&self, agent_id: AgentId) {
        self.inner.conns.lock().unwrap().remove(&agent_id);
    }

    pub fn decider(&self) -> &std::sync::Arc<ac_decider::DeciderChain> {
        &self.inner.decider
    }
}

impl WakeEngine {
    /// A reply handle not tied to a particular connection (HTTP replies, webhooks).
    pub fn replier_for(&self, agent_id: AgentId) -> AgentReplier {
        AgentReplier { agent_id, conn_id: 0, engine: self.clone() }
    }
}
