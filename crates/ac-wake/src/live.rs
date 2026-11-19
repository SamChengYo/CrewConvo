//! Handing a queued wake to a run that is already going.
//!
//! An agent handles one wake at a time, so a message that arrives while it works waits in the
//! queue and the agent only learns of it after it has finished. A transport that can talk to
//! the running agent (a local coding agent whose command-line tool accepts more input) takes
//! the waiting wake out of the queue instead and delivers its messages right away. Which
//! messages wake the agent is decided as always; only the moment of delivery changes.

use ac_core::model::{Wake, WakeStatus};
use ac_core::protocol::WakePayload;
use ac_core::{AgentId, AppResult};
use uuid::Uuid;

use crate::WakeEngine;

impl WakeEngine {
    /// The wakes of this agent that wait in one conversation, oldest first. Discussion turns
    /// are left out: they are steps of a moderated exchange, not messages to pass along.
    pub async fn waiting_wakes(&self, agent_id: AgentId, conversation_id: Uuid) -> AppResult<Vec<Wake>> {
        let all = self.inner.store.active_wakes_for_conversation(conversation_id).await?;
        Ok(all
            .into_iter()
            .filter(|w| w.agent_id == agent_id && w.status == WakeStatus::Pending && w.discussion_id.is_none())
            .collect())
    }

    /// Takes a waiting wake out of the queue and returns what the agent has not seen yet.
    /// None when it is no longer waiting (cancelled, or the queue got to it first). From here
    /// on the wake is the caller's: it must end with [`WakeEngine::settle_taken`].
    pub async fn take_waiting(&self, wake: &Wake) -> AppResult<Option<WakePayload>> {
        let store = &self.inner.store;
        let Some(agent) = store.get_agent(wake.agent_id).await? else { return Ok(None) };
        let Some(taken) = store.transition_wake(wake.id, &[WakeStatus::Pending], WakeStatus::Delivered).await? else {
            return Ok(None);
        };
        self.publish_wake(&taken).await;
        match self.inner.context.assemble(&taken, &agent).await {
            Ok(payload) => {
                if let Ok(v) = serde_json::to_value(&payload) {
                    let _ = store.set_wake_payload(wake.id, &v).await;
                }
                Ok(Some(payload))
            }
            Err(e) => {
                // Could not be read: back into the queue, to be delivered the ordinary way.
                self.settle_taken(wake, 0, None).await?;
                Err(e)
            }
        }
    }

    /// Ends a wake taken with [`WakeEngine::take_waiting`]. `delivered` is the note kept as its
    /// private log when the running agent got the messages; None puts the wake back in the
    /// queue, so that it is delivered after the run — a message is never dropped and never
    /// delivered twice.
    pub async fn settle_taken(&self, wake: &Wake, max_seq: i64, delivered: Option<&str>) -> AppResult<()> {
        let store = &self.inner.store;
        match delivered {
            Some(note) => {
                store.update_cursor(wake.conversation_id, wake.agent_id, max_seq).await?;
                self.finish_and_publish(wake.id, WakeStatus::Done, Some(note), None).await?;
            }
            None => {
                if let Some(w) = store.transition_wake(wake.id, &[WakeStatus::Delivered], WakeStatus::Pending).await? {
                    self.publish_wake(&w).await;
                }
                self.kick_worker(wake.agent_id);
            }
        }
        Ok(())
    }
}
