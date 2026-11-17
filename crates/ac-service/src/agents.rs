use ac_core::bus::RealtimeEvent;
use ac_core::mention::validate_handle;
use ac_core::model::{Agent, AgentOrigin, AgentProfile, AgentStatus, Member};
use ac_core::{AppError, AppResult};
use ac_store::{AgentPatch, NewAgent};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{Service, require_human};

pub const MAX_WAKE_SECONDS_CAP: i32 = 30 * 60;

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAgentInput {
    pub handle: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub description: String,
    #[serde(default)]
    pub expertise: Vec<String>,
    pub model: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Allowed tool names; empty = all chat tools.
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub max_wake_seconds: Option<i32>,
    #[serde(default)]
    pub daily_token_budget: Option<i64>,
    #[serde(default)]
    pub sandbox_config: Option<Value>,
    /// Channels to add the agent to right away.
    #[serde(default)]
    pub join_conversations: Vec<Uuid>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateAgentInput {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub expertise: Option<Vec<String>>,
    pub status: Option<AgentStatus>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub tools: Option<Vec<String>>,
    pub max_wake_seconds: Option<i32>,
    /// JSON null clears the budget.
    #[serde(default, deserialize_with = "double_option")]
    pub daily_token_budget: Option<Option<i64>>,
}

fn double_option<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<i64>>, D::Error> {
    Ok(Some(Option::<i64>::deserialize(d)?))
}

fn clean_expertise(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = v.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    out.dedup();
    out.truncate(20);
    out
}

fn check_wake_seconds(s: i32) -> AppResult<i32> {
    if !(10..=MAX_WAKE_SECONDS_CAP).contains(&s) {
        return Err(AppError::bad_request(format!("max_wake_seconds 需介於 10 與 {MAX_WAKE_SECONDS_CAP}")));
    }
    Ok(s)
}

impl Service {
    pub async fn create_builtin_agent(&self, owner: &Member, input: CreateAgentInput) -> AppResult<Agent> {
        require_human(owner)?;
        let handle = input.handle.trim().to_string();
        validate_handle(&handle).map_err(AppError::BadRequest)?;
        if input.description.trim().is_empty() {
            return Err(AppError::bad_request("description 必填（其他 Agent 與 Decider 依此判斷）"));
        }
        if input.model.trim().is_empty() {
            return Err(AppError::bad_request("model 必填"));
        }
        let agent = self
            .inner
            .store
            .create_agent(&NewAgent {
                workspace_id: owner.workspace_id,
                display_name: input.display_name.filter(|d| !d.trim().is_empty()).unwrap_or_else(|| handle.clone()),
                handle,
                origin: AgentOrigin::Builtin,
                owner_id: owner.id,
                description: input.description.trim().to_string(),
                expertise: clean_expertise(&input.expertise),
                model: Some(input.model.trim().to_string()),
                system_prompt: input.system_prompt,
                tools: Value::from(input.tools),
                max_wake_seconds: check_wake_seconds(input.max_wake_seconds.unwrap_or(300))?,
                daily_token_budget: input.daily_token_budget,
                sandbox_config: input.sandbox_config.clone().unwrap_or_else(|| serde_json::json!({})),
            })
            .await
            .map_err(|e| match e {
                AppError::Conflict(_) => AppError::Conflict("handle 已被使用".into()),
                other => other,
            })?;
        for conv in input.join_conversations {
            let c = self.member_conversation(owner, conv).await?;
            self.inner.store.add_conversation_member(c.id, agent.member.id, Some(owner.id)).await?;
            self.inner.engine.bus().publish(RealtimeEvent::MembershipChanged {
                conversation_id: c.id,
                member_id: agent.member.id,
                joined: true,
            });
        }
        self.inner.engine.bus().publish(RealtimeEvent::AgentsChanged { workspace_id: owner.workspace_id });
        self.notify_agent_changed(&agent);
        Ok(agent)
    }

    pub async fn get_agent(&self, actor: &Member, id: Uuid) -> AppResult<Agent> {
        let agent = self.inner.store.get_agent(id).await?.ok_or_else(|| AppError::not_found("agent"))?;
        if agent.member.workspace_id != actor.workspace_id {
            return Err(AppError::not_found("agent"));
        }
        Ok(agent)
    }

    /// Full agent records (for the management UI).
    pub async fn list_agent_records(&self, actor: &Member) -> AppResult<Vec<Agent>> {
        self.inner.store.list_agents(actor.workspace_id, None, None).await
    }

    /// `list_agents` tool: public profiles, optionally filtered.
    pub async fn list_agents(
        &self,
        actor: &Member,
        query: Option<&str>,
        conversation_id: Option<Uuid>,
    ) -> AppResult<Vec<AgentProfile>> {
        if let Some(c) = conversation_id {
            self.member_conversation(actor, c).await?;
        }
        let mut agents = self.inner.store.list_agents(actor.workspace_id, query, conversation_id).await?;
        // Agents only discover agents of their own owner and of their groups (members of a
        // conversation they are in are listed when asking about that conversation).
        if conversation_id.is_none()
            && let Some(visible) = self.visible_agents_for(actor).await?
        {
            agents.retain(|a| visible.contains(&a.member.id));
        }
        Ok(agents.iter().map(AgentProfile::from).collect())
    }

    pub async fn update_agent(&self, actor: &Member, id: Uuid, input: UpdateAgentInput) -> AppResult<Agent> {
        require_human(actor)?;
        let agent = self.get_agent(actor, id).await?;
        if agent.owner_id != actor.id && !actor.is_admin {
            return Err(AppError::forbidden("只有擁有者或管理員可以修改此 Agent"));
        }
        if let Some(d) = &input.description
            && d.trim().is_empty()
        {
            return Err(AppError::bad_request("description 不能是空的"));
        }
        let patch = AgentPatch {
            display_name: input.display_name.filter(|d| !d.trim().is_empty()),
            description: input.description.map(|d| d.trim().to_string()),
            expertise: input.expertise.as_deref().map(clean_expertise),
            status: input.status,
            model: input.model.filter(|m| !m.trim().is_empty()),
            system_prompt: input.system_prompt,
            tools: input.tools.map(Value::from),
            max_wake_seconds: input.max_wake_seconds.map(check_wake_seconds).transpose()?,
            daily_token_budget: input.daily_token_budget,
        };
        let updated = self.inner.store.update_agent(id, &patch).await?;
        self.inner.engine.bus().publish(RealtimeEvent::AgentsChanged { workspace_id: actor.workspace_id });
        self.notify_agent_changed(&updated);
        Ok(updated)
    }
}
