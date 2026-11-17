//! Wake engine (spec 6): rule layer, per-agent wake queues, depth/budget control, stop.
//!
//! The platform decides *whom* to wake; the agent decides whether to speak.
//! Every agent (builtin or external) receives wakes through an [`AgentLink`] and answers
//! with [`AgentReply`] messages — there is no privileged path for builtin agents.

mod decide;
mod discussion;
mod hub;
mod rules;
mod worker;

pub use hub::{AgentLink, AgentReplier};

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ac_context::ContextAssembler;
use ac_core::bus::{Bus, RealtimeEvent};
use ac_core::limits::Limits;
use ac_core::model::{Message, MessageView, Wake, WakeStatus};
use ac_core::protocol::AgentEvent;
use ac_core::{AgentId, AppError, AppResult, WakeId};
use ac_decider::DeciderChain;
use ac_store::{NewMessage, Store};
use chrono::{DateTime, NaiveDate, Utc};
use tokio::sync::mpsc;
use uuid::Uuid;

pub(crate) enum WakeSignal {
    Reply(ac_core::protocol::AgentReply),
    Cancel,
}

pub(crate) struct Inner {
    pub store: Store,
    pub bus: Bus,
    pub limits: Limits,
    pub context: ContextAssembler,
    pub input: mpsc::UnboundedSender<Message>,
    pub queued: AtomicUsize,
    /// background decider tasks in flight
    pub background: AtomicUsize,
    /// agent → live connection
    pub conns: Mutex<HashMap<AgentId, hub::Conn>>,
    pub next_conn_id: AtomicUsize,
    /// agents that currently have a worker task
    pub workers: tokio::sync::Mutex<HashSet<AgentId>>,
    /// in-flight wakes → signal channel of the worker handling it
    pub inflight: Mutex<HashMap<WakeId, mpsc::UnboundedSender<WakeSignal>>>,
    /// (agent, day) pairs for which a "daily budget exhausted" notice was posted
    pub budget_notices: Mutex<HashSet<(AgentId, NaiveDate)>>,
    pub decider: Arc<DeciderChain>,
    /// serializes discussion state changes
    pub discussions: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct WakeEngine {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) fn start_of_day(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive().and_hms_opt(0, 0, 0).expect("midnight").and_utc()
}

impl WakeEngine {
    /// Creates the engine and spawns its message-processing loop.
    pub fn start(
        store: Store,
        bus: Bus,
        limits: Limits,
        context: ContextAssembler,
        decider: Arc<DeciderChain>,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let engine = WakeEngine {
            inner: Arc::new(Inner {
                store,
                bus,
                limits,
                context,
                input: tx,
                queued: AtomicUsize::new(0),
                background: AtomicUsize::new(0),
                conns: Mutex::new(HashMap::new()),
                next_conn_id: AtomicUsize::new(1),
                workers: tokio::sync::Mutex::new(HashSet::new()),
                inflight: Mutex::new(HashMap::new()),
                budget_notices: Mutex::new(HashSet::new()),
                decider,
                discussions: tokio::sync::Mutex::new(()),
            }),
        };
        let e = engine.clone();
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let id = msg.id;
                if let Err(err) = e.process_message(msg).await {
                    tracing::error!(message_id = %id, error = %err, "wake engine failed to process message");
                }
                e.inner.queued.fetch_sub(1, Ordering::SeqCst);
            }
        });
        engine
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }
    pub fn bus(&self) -> &Bus {
        &self.inner.bus
    }
    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    /// Hands a freshly written message to the engine. Never blocks the write path.
    pub fn submit(&self, msg: Message) {
        self.inner.queued.fetch_add(1, Ordering::SeqCst);
        if self.inner.input.send(msg).is_err() {
            self.inner.queued.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// True when no submitted message is waiting to be processed.
    pub fn is_input_idle(&self) -> bool {
        self.inner.queued.load(Ordering::SeqCst) == 0 && self.inner.background.load(Ordering::SeqCst) == 0
    }

    /// Re-dispatches pending wakes left by a previous process and fails orphaned in-flight ones.
    pub async fn recover(&self) -> AppResult<()> {
        let n = self.inner.store.fail_orphaned_wakes().await?;
        if n > 0 {
            tracing::warn!(count = n, "marked orphaned in-flight wakes as failed");
        }
        for agent in self.inner.store.agents_with_pending_wakes().await? {
            self.ensure_worker(agent).await;
        }
        Ok(())
    }

    /// Writes a message and publishes it to realtime subscribers. Does not submit it.
    pub async fn insert_and_publish(&self, n: &NewMessage) -> AppResult<(Message, MessageView)> {
        let msg = self.inner.store.insert_message(n).await?;
        // The author has read everything up to its own message.
        self.inner.store.update_cursor(msg.conversation_id, msg.author_id, msg.seq).await?;
        let view = self
            .inner
            .store
            .get_message_view(msg.id)
            .await?
            .ok_or_else(|| AppError::internal("inserted message vanished"))?;
        self.inner.bus.publish(RealtimeEvent::MessageCreated { message: view.clone() });
        Ok((msg, view))
    }

    /// Posts a platform notice into a conversation (never wakes anybody).
    pub async fn post_system(
        &self,
        conversation_id: Uuid,
        thread_root_id: Option<Uuid>,
        body: String,
    ) -> AppResult<()> {
        let conv = self
            .inner
            .store
            .get_conversation(conversation_id)
            .await?
            .ok_or_else(|| AppError::not_found("conversation"))?;
        let system = self.inner.store.system_member(conv.workspace_id).await?;
        self.insert_and_publish(&NewMessage {
            conversation_id,
            thread_root_id,
            author_id: system.id,
            body,
            mentions: vec![],
            depth: 0,
            chain_root_id: None,
            wake_id: None,
        })
        .await?;
        Ok(())
    }

    pub(crate) async fn publish_wake(&self, wake: &Wake) {
        let handle = match self.inner.store.get_member(wake.agent_id).await {
            Ok(Some(m)) => m.handle,
            _ => String::new(),
        };
        self.inner.bus.publish(RealtimeEvent::WakeStatus {
            conversation_id: wake.conversation_id,
            wake_id: wake.id,
            agent_id: wake.agent_id,
            agent_handle: handle,
            thread_root_id: wake.thread_root_id,
            reason: wake.reason,
            mode: wake.mode,
            status: wake.status,
        });
    }

    /// Validates that `wake_id` belongs to `agent_id` and is in flight; returns the wake.
    pub async fn active_wake_for(&self, wake_id: WakeId, agent_id: AgentId) -> AppResult<Wake> {
        let wake = self.inner.store.get_wake(wake_id).await?.ok_or_else(|| AppError::not_found("wake"))?;
        if wake.agent_id != agent_id {
            return Err(AppError::forbidden("wake 不屬於此 Agent"));
        }
        if !wake.status.is_in_flight() {
            return Err(AppError::forbidden(format!("wake 已結束（{}），不能再以此 wake 發言", wake.status)));
        }
        Ok(wake)
    }

    /// Human "stop" (spec 6.7): cancel pending/running wakes, freeze the conversation.
    pub async fn stop_conversation(&self, conversation_id: Uuid) -> AppResult<Vec<Wake>> {
        let conv = self.inner.store.set_frozen(conversation_id, true).await?;
        self.inner.bus.publish(RealtimeEvent::ConversationUpdated { conversation: conv });
        self.cancel_discussions(conversation_id).await?;
        let cancelled = self.inner.store.cancel_conversation_wakes(conversation_id).await?;
        let mut out = Vec::new();
        for (wake, old) in cancelled {
            if old.is_in_flight() {
                self.signal(wake.id, WakeSignal::Cancel);
            }
            self.publish_wake(&wake).await;
            out.push(wake);
        }
        Ok(out)
    }

    pub(crate) fn signal(&self, wake_id: WakeId, sig: WakeSignal) -> bool {
        let tx = self.inner.inflight.lock().unwrap().get(&wake_id).cloned();
        match tx {
            Some(tx) => tx.send(sig).is_ok(),
            None => false,
        }
    }

    /// Sends an event to an agent's live connection.
    pub(crate) fn send_event(&self, agent_id: AgentId, ev: AgentEvent) -> bool {
        let conns = self.inner.conns.lock().unwrap();
        match conns.get(&agent_id) {
            Some(c) => c.events.send(ev).is_ok(),
            None => false,
        }
    }

    pub fn is_connected(&self, agent_id: AgentId) -> bool {
        let conns = self.inner.conns.lock().unwrap();
        conns.get(&agent_id).map(|c| !c.events.is_closed()).unwrap_or(false)
    }

    /// Active (pending / in-flight) wakes of a conversation.
    pub async fn active_wakes(&self, conversation_id: Uuid) -> AppResult<Vec<Wake>> {
        self.inner.store.active_wakes_for_conversation(conversation_id).await
    }

    pub(crate) async fn finish_and_publish(
        &self,
        wake_id: WakeId,
        status: WakeStatus,
        private_log: Option<&str>,
        tokens: Option<i64>,
    ) -> AppResult<Option<Wake>> {
        let w = self.inner.store.finish_wake(wake_id, status, private_log, tokens).await?;
        if let Some(w) = &w {
            self.publish_wake(w).await;
        }
        Ok(w)
    }
}
