//! External agents (spec 5, phase 4): tokens (hash only, rotatable, revocable), delivery
//! settings, connection status; plus agent deletion and sandbox limits.

use ac_core::bus::RealtimeEvent;
use ac_core::mention::validate_handle;
use ac_core::model::{Agent, AgentOrigin, AgentStatus, Member};
use ac_core::{AppError, AppResult};
use ac_decider::net;
use ac_sandbox::SandboxLimits;
use ac_store::{AgentDelivery, NewAgent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::{hash_token, new_token};
use crate::{Service, require_human};

pub const TOKEN_PREFIX: &str = "ccat_";

#[derive(Debug, Clone, Deserialize)]
pub struct CreateExternalAgentInput {
    pub handle: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub description: String,
    #[serde(default)]
    pub expertise: Vec<String>,
    /// `websocket` (default) or `webhook`.
    #[serde(default)]
    pub delivery: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub max_wake_seconds: Option<i32>,
    #[serde(default)]
    pub daily_token_budget: Option<i64>,
    #[serde(default)]
    pub join_conversations: Vec<Uuid>,
}

/// Returned once at creation / rotation. Never retrievable again.
#[derive(Debug, Clone, Serialize)]
pub struct AgentCredentials {
    pub agent: Agent,
    pub token: Option<String>,
    pub webhook_secret: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeliveryInput {
    pub delivery: String,
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Generate a new webhook secret.
    #[serde(default)]
    pub rotate_secret: bool,
}

fn new_agent_token() -> String {
    format!("{TOKEN_PREFIX}{}", new_token())
}

impl Service {
    fn can_manage(actor: &Member, agent: &Agent) -> AppResult<()> {
        require_human(actor)?;
        if agent.owner_id != actor.id && !actor.is_admin {
            return Err(AppError::forbidden("只有擁有者或管理員可以管理此 Agent"));
        }
        Ok(())
    }

    async fn validate_delivery(&self, delivery: &str, url: Option<&str>) -> AppResult<()> {
        match delivery {
            "websocket" => Ok(()),
            "webhook" => {
                let u = url
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .ok_or_else(|| AppError::bad_request("webhook 模式需要 webhook_url"))?;
                net::check_url(u, self.egress()).await.map_err(AppError::BadRequest)?;
                Ok(())
            }
            _ => Err(AppError::bad_request("delivery 只能是 websocket 或 webhook")),
        }
    }

    pub async fn create_external_agent(
        &self,
        owner: &Member,
        i: CreateExternalAgentInput,
    ) -> AppResult<AgentCredentials> {
        require_human(owner)?;
        let handle = i.handle.trim().to_string();
        validate_handle(&handle).map_err(AppError::BadRequest)?;
        if i.description.trim().is_empty() {
            return Err(AppError::bad_request("description 必填（其他 Agent 與 Decider 依此判斷）"));
        }
        let delivery = i.delivery.clone().unwrap_or_else(|| "websocket".into());
        self.validate_delivery(&delivery, i.webhook_url.as_deref()).await?;
        let agent = self
            .inner
            .store
            .create_agent(&NewAgent {
                workspace_id: owner.workspace_id,
                display_name: i.display_name.clone().filter(|d| !d.trim().is_empty()).unwrap_or_else(|| handle.clone()),
                handle,
                origin: AgentOrigin::External,
                owner_id: owner.id,
                description: i.description.trim().to_string(),
                expertise: i.expertise.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                model: None,
                system_prompt: None,
                tools: json!([]),
                max_wake_seconds: i.max_wake_seconds.unwrap_or(300).clamp(10, crate::agents::MAX_WAKE_SECONDS_CAP),
                daily_token_budget: i.daily_token_budget,
                sandbox_config: json!({}),
            })
            .await
            .map_err(|e| match e {
                AppError::Conflict(_) => AppError::Conflict("handle 已被使用".into()),
                other => other,
            })?;
        // Offline until it connects (webhook agents are always reachable).
        self.inner
            .store
            .set_agent_status(
                agent.member.id,
                if delivery == "webhook" { AgentStatus::Online } else { AgentStatus::Offline },
            )
            .await?;
        let webhook_secret = if delivery == "webhook" { Some(new_token()) } else { None };
        self.inner
            .store
            .set_agent_delivery(
                agent.member.id,
                &AgentDelivery {
                    delivery: delivery.clone(),
                    webhook_url: i.webhook_url.clone().filter(|_| delivery == "webhook"),
                    webhook_secret_encrypted: webhook_secret.as_deref().map(|s| self.secrets().encrypt(s)),
                },
            )
            .await?;
        let token = new_agent_token();
        self.inner.store.set_agent_token(agent.member.id, Some(&hash_token(&token)), Some(&token[..12])).await?;
        for conv in i.join_conversations {
            let c = self.member_conversation(owner, conv).await?;
            if self.inner.store.add_conversation_member(c.id, agent.member.id, Some(owner.id)).await? {
                self.inner.engine.bus().publish(RealtimeEvent::MembershipChanged {
                    conversation_id: c.id,
                    member_id: agent.member.id,
                    joined: true,
                });
            }
        }
        let agent = self.get_agent(owner, agent.member.id).await?;
        self.inner.engine.bus().publish(RealtimeEvent::AgentsChanged { workspace_id: owner.workspace_id });
        self.notify_agent_changed(&agent);
        Ok(AgentCredentials { agent, token: Some(token), webhook_secret })
    }

    pub async fn rotate_agent_token(&self, actor: &Member, id: Uuid) -> AppResult<AgentCredentials> {
        let agent = self.get_agent(actor, id).await?;
        Service::can_manage(actor, &agent)?;
        let token = new_agent_token();
        self.inner.store.set_agent_token(id, Some(&hash_token(&token)), Some(&token[..12])).await?;
        // Existing connections authenticated with the old token are dropped.
        self.inner.engine.disconnect_agent(id);
        let agent = self.get_agent(actor, id).await?;
        Ok(AgentCredentials { agent, token: Some(token), webhook_secret: None })
    }

    pub async fn revoke_agent_token(&self, actor: &Member, id: Uuid) -> AppResult<Agent> {
        let agent = self.get_agent(actor, id).await?;
        Service::can_manage(actor, &agent)?;
        self.inner.store.set_agent_token(id, None, None).await?;
        self.inner.engine.disconnect_agent(id);
        self.get_agent(actor, id).await
    }

    pub async fn update_agent_delivery(
        &self,
        actor: &Member,
        id: Uuid,
        i: DeliveryInput,
    ) -> AppResult<AgentCredentials> {
        let agent = self.get_agent(actor, id).await?;
        Service::can_manage(actor, &agent)?;
        if agent.origin != AgentOrigin::External {
            return Err(AppError::bad_request("只有外部 Agent 有傳送方式設定"));
        }
        self.validate_delivery(&i.delivery, i.webhook_url.as_deref()).await?;
        let current = self.inner.store.agent_delivery(id).await?;
        let mut secret_plain = None;
        let secret_enc = if i.delivery == "webhook" {
            match current.as_ref().and_then(|c| c.webhook_secret_encrypted.clone()) {
                Some(enc) if !i.rotate_secret => Some(enc),
                _ => {
                    let s = new_token();
                    let enc = self.secrets().encrypt(&s);
                    secret_plain = Some(s);
                    Some(enc)
                }
            }
        } else {
            None
        };
        self.inner
            .store
            .set_agent_delivery(
                id,
                &AgentDelivery {
                    delivery: i.delivery.clone(),
                    webhook_url: i.webhook_url.clone().filter(|_| i.delivery == "webhook"),
                    webhook_secret_encrypted: secret_enc,
                },
            )
            .await?;
        if i.delivery == "webhook" {
            self.inner.store.set_agent_status(id, AgentStatus::Online).await?;
        }
        self.inner.engine.disconnect_agent(id);
        let agent = self.get_agent(actor, id).await?;
        self.notify_agent_changed(&agent);
        Ok(AgentCredentials { agent, token: None, webhook_secret: secret_plain })
    }

    /// Bearer token → agent member (spec 5: token hashes only).
    pub async fn authenticate_agent(&self, token: &str) -> AppResult<(Member, Agent)> {
        let token = token.trim();
        if !token.starts_with(TOKEN_PREFIX) {
            return Err(AppError::Unauthorized);
        }
        let agent = self.inner.store.agent_by_token_hash(&hash_token(token)).await?.ok_or(AppError::Unauthorized)?;
        if agent.status == AgentStatus::Disabled {
            return Err(AppError::forbidden("此 Agent 已停用"));
        }
        Ok((agent.member.clone(), agent))
    }

    /// Connection status of an external WebSocket agent.
    pub async fn agent_connection_changed(&self, agent: &Agent, connected: bool) -> AppResult<()> {
        if agent.origin != AgentOrigin::External {
            return Ok(());
        }
        self.inner.store.touch_agent_seen(agent.member.id).await?;
        let current = self.inner.store.get_agent(agent.member.id).await?;
        if let Some(a) = current
            && a.status != AgentStatus::Disabled
            && a.delivery.as_deref() != Some("webhook")
        {
            self.inner
                .store
                .set_agent_status(a.member.id, if connected { AgentStatus::Online } else { AgentStatus::Offline })
                .await?;
            self.inner.engine.bus().publish(RealtimeEvent::AgentsChanged { workspace_id: a.member.workspace_id });
        }
        Ok(())
    }

    /// Decrypted webhook target of an agent.
    pub async fn webhook_target(&self, agent_id: Uuid) -> AppResult<Option<(String, String)>> {
        let Some(d) = self.inner.store.agent_delivery(agent_id).await? else { return Ok(None) };
        if d.delivery != "webhook" {
            return Ok(None);
        }
        match (d.webhook_url, d.webhook_secret_encrypted) {
            (Some(u), Some(enc)) => Ok(Some((u, self.secrets().decrypt(&enc).map_err(AppError::Internal)?))),
            _ => Ok(None),
        }
    }

    pub async fn set_sandbox_limits(&self, actor: &Member, id: Uuid, cfg: Value) -> AppResult<Agent> {
        let agent = self.get_agent(actor, id).await?;
        Service::can_manage(actor, &agent)?;
        if agent.origin != AgentOrigin::Builtin {
            return Err(AppError::bad_request("只有內建 Agent 有沙箱"));
        }
        let limits: SandboxLimits =
            serde_json::from_value(cfg).map_err(|e| AppError::bad_request(format!("sandbox_config 格式錯誤：{e}")))?;
        if limits.network == ac_sandbox::NetworkPolicy::Open && !actor.is_admin {
            return Err(AppError::forbidden("只有管理員可以開放沙箱網路"));
        }
        self.inner.store.set_sandbox_config(id, &serde_json::to_value(&limits).unwrap_or_default()).await?;
        // Limits apply when the container is recreated.
        if let Some(sb) = &self.inner.sandbox {
            let _ = sb.stop(id).await;
        }
        self.get_agent(actor, id).await
    }

    /// Deletes an agent. Its sandbox volume is destroyed — irreversible, so `confirm` is required.
    pub async fn delete_agent(&self, actor: &Member, id: Uuid, confirm: bool) -> AppResult<()> {
        let agent = self.get_agent(actor, id).await?;
        Service::can_manage(actor, &agent)?;
        if !confirm {
            return Err(AppError::bad_request(
                "刪除 Agent 會一併刪除它的沙箱與 /workspace 資料，請確認（confirm=true）",
            ));
        }
        self.inner.store.deactivate_agent(id).await?;
        self.inner.engine.disconnect_agent(id);
        if agent.origin == AgentOrigin::Builtin
            && let Some(sb) = &self.inner.sandbox
            && let Err(e) = sb.destroy(id).await
        {
            tracing::warn!(agent = %id, error = %e, "sandbox destroy failed");
        }
        if let Ok(Some(a)) = self.inner.store.get_agent(id).await {
            self.notify_agent_changed(&a);
        }
        self.inner.engine.bus().publish(RealtimeEvent::AgentsChanged { workspace_id: actor.workspace_id });
        Ok(())
    }
}
