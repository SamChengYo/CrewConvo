//! Business logic shared by every entry point: human REST API, agent REST/MCP endpoints
//! and the builtin runtime. Permission checks and rate limits live here, so no caller has
//! a privileged path.

pub mod agents;
pub mod auth;
pub mod connect;
pub mod conversations;
pub mod deciders;
pub mod external;
pub mod groups;
pub mod messages;
pub mod observe;
pub mod provider;
mod ratelimit;
pub mod tools;

use std::sync::{Arc, RwLock};

use ac_core::limits::Limits;
use ac_core::model::{Agent, Conversation, Member, MemberKind};
use ac_core::{AppError, AppResult};
use ac_decider::{DeciderChain, EgressPolicy, SecretBox};
use ac_sandbox::Sandbox;
use ac_search::SearchService;
use ac_store::Store;
use ac_wake::WakeEngine;
use uuid::Uuid;

pub use ratelimit::RateLimiter;

/// Notified when agents are created, changed or deleted (the runtime / webhook connectors
/// attach to them).
pub trait AgentLifecycle: Send + Sync {
    fn agent_changed(&self, agent: &Agent);
}

pub(crate) struct Inner {
    pub store: Store,
    pub engine: WakeEngine,
    pub limits: Limits,
    pub rate: RateLimiter,
    pub lifecycle: RwLock<Vec<Arc<dyn AgentLifecycle>>>,
    pub search: SearchService,
    pub sandbox: Option<Arc<dyn Sandbox>>,
    pub decider: Arc<DeciderChain>,
}

#[derive(Clone)]
pub struct Service {
    pub(crate) inner: Arc<Inner>,
}

impl Service {
    pub fn new(
        store: Store,
        engine: WakeEngine,
        search: SearchService,
        sandbox: Option<Arc<dyn Sandbox>>,
        decider: Arc<DeciderChain>,
    ) -> Self {
        let limits = engine.limits().clone();
        Service {
            inner: Arc::new(Inner {
                store,
                engine,
                limits,
                rate: RateLimiter::default(),
                lifecycle: RwLock::new(Vec::new()),
                search,
                sandbox,
                decider,
            }),
        }
    }

    pub fn search(&self) -> &SearchService {
        &self.inner.search
    }
    pub fn sandbox(&self) -> Option<&Arc<dyn Sandbox>> {
        self.inner.sandbox.as_ref()
    }
    pub fn secrets(&self) -> &SecretBox {
        self.inner.decider.secrets()
    }
    pub fn egress(&self) -> EgressPolicy {
        self.inner.decider.policy()
    }

    pub fn add_lifecycle(&self, l: Arc<dyn AgentLifecycle>) {
        self.inner.lifecycle.write().unwrap().push(l);
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }
    pub fn engine(&self) -> &WakeEngine {
        &self.inner.engine
    }
    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    pub(crate) fn notify_agent_changed(&self, agent: &Agent) {
        let hooks: Vec<Arc<dyn AgentLifecycle>> = self.inner.lifecycle.read().unwrap().clone();
        for l in hooks {
            l.agent_changed(agent);
        }
    }

    pub(crate) fn require_admin(actor: &Member) -> AppResult<()> {
        if actor.kind != MemberKind::Human || !actor.is_admin {
            return Err(AppError::forbidden("只有 workspace 管理員可以執行此操作"));
        }
        Ok(())
    }

    /// Loads a conversation the actor can see (member, or public channel of the same workspace).
    pub(crate) async fn visible_conversation(&self, actor: &Member, id: Uuid) -> AppResult<(Conversation, bool)> {
        let conv = self.inner.store.get_conversation(id).await?.ok_or_else(|| AppError::not_found("conversation"))?;
        if conv.workspace_id != actor.workspace_id {
            return Err(AppError::not_found("conversation"));
        }
        let member = self.inner.store.is_member(id, actor.id).await?;
        Ok((conv, member))
    }

    /// Loads a conversation the actor is a member of.
    pub(crate) async fn member_conversation(&self, actor: &Member, id: Uuid) -> AppResult<Conversation> {
        let (conv, member) = self.visible_conversation(actor, id).await?;
        if !member {
            return Err(AppError::forbidden("你不是此對話的成員"));
        }
        Ok(conv)
    }
}

pub(crate) fn require_human(actor: &Member) -> AppResult<()> {
    if actor.kind != MemberKind::Human {
        return Err(AppError::forbidden("只有人類成員可以執行此操作"));
    }
    Ok(())
}
