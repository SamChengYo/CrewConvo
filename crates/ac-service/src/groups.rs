//! Agent groups: which agents an agent can discover.
//!
//! An agent's directory — `list_agents`, and the agents it may `invite` or `open_dm` — is limited to
//! agents with the same owner plus agents that share a group with it. Members of a conversation it
//! is in stay visible there (they are in the room anyway). Humans see every agent.
//!
//! Any human can create a group. Its creator and admins manage it and can add any agent.

use ac_core::model::{Member, MemberKind};
use ac_core::{AppError, AppResult};
use ac_store::AgentGroup;
use serde::Deserialize;
use uuid::Uuid;

use crate::{Service, require_human};

#[derive(Debug, Clone, Deserialize)]
pub struct GroupInput {
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub agent_ids: Option<Vec<Uuid>>,
}

impl Service {
    pub async fn list_agent_groups(&self, actor: &Member) -> AppResult<Vec<AgentGroup>> {
        require_human(actor)?;
        self.inner.store.list_agent_groups(actor.workspace_id).await
    }

    async fn group_for_edit(&self, actor: &Member, id: Uuid) -> AppResult<AgentGroup> {
        require_human(actor)?;
        let g = self
            .inner
            .store
            .get_agent_group(id)
            .await?
            .filter(|g| g.workspace_id == actor.workspace_id)
            .ok_or_else(|| AppError::not_found("群組"))?;
        if g.created_by != actor.id && !actor.is_admin {
            return Err(AppError::forbidden("只有建立者或管理員可以修改這個群組"));
        }
        Ok(g)
    }

    async fn check_group_agents(&self, actor: &Member, ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
        let mut out = Vec::new();
        for id in ids {
            let a = self.inner.store.get_agent(*id).await?.ok_or_else(|| AppError::bad_request("找不到 Agent"))?;
            if a.member.workspace_id != actor.workspace_id || a.member.kind != MemberKind::Agent {
                return Err(AppError::bad_request("找不到 Agent"));
            }
            if !out.contains(id) {
                out.push(*id);
            }
        }
        Ok(out)
    }

    fn clean_name(name: &str) -> AppResult<String> {
        let n = name.trim();
        if n.is_empty() || n.chars().count() > 60 {
            return Err(AppError::bad_request("群組名稱需為 1–60 個字"));
        }
        Ok(n.to_string())
    }

    pub async fn create_agent_group(&self, actor: &Member, i: GroupInput) -> AppResult<AgentGroup> {
        require_human(actor)?;
        let name = Self::clean_name(i.name.as_deref().unwrap_or(""))?;
        let agents = self.check_group_agents(actor, i.agent_ids.as_deref().unwrap_or(&[])).await?;
        let id = self
            .inner
            .store
            .create_agent_group(actor.workspace_id, &name, i.description.as_deref().unwrap_or("").trim(), actor.id)
            .await
            .map_err(|e| match e {
                AppError::Conflict(_) => AppError::bad_request(format!("已經有叫「{name}」的群組")),
                e => e,
            })?;
        self.inner.store.set_agent_group_members(id, &agents, actor.id).await?;
        self.inner.store.get_agent_group(id).await?.ok_or_else(|| AppError::not_found("群組"))
    }

    pub async fn update_agent_group(&self, actor: &Member, id: Uuid, i: GroupInput) -> AppResult<AgentGroup> {
        self.group_for_edit(actor, id).await?;
        let name = i.name.as_deref().map(Self::clean_name).transpose()?;
        self.inner
            .store
            .update_agent_group(id, name.as_deref(), i.description.as_deref().map(str::trim))
            .await
            .map_err(|e| match e {
                AppError::Conflict(_) => AppError::bad_request("已經有同名的群組"),
                e => e,
            })?;
        if let Some(ids) = &i.agent_ids {
            let agents = self.check_group_agents(actor, ids).await?;
            self.inner.store.set_agent_group_members(id, &agents, actor.id).await?;
        }
        self.inner.store.get_agent_group(id).await?.ok_or_else(|| AppError::not_found("群組"))
    }

    pub async fn delete_agent_group(&self, actor: &Member, id: Uuid) -> AppResult<()> {
        self.group_for_edit(actor, id).await?;
        self.inner.store.delete_agent_group(id).await
    }

    /// None = everything is visible (humans).
    pub(crate) async fn visible_agents_for(&self, actor: &Member) -> AppResult<Option<Vec<Uuid>>> {
        if actor.kind != MemberKind::Agent {
            return Ok(None);
        }
        Ok(Some(self.inner.store.visible_agent_ids(actor.id).await?))
    }
}
