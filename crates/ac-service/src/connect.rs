//! `crewconvo setup <server>`: browser approval instead of invite codes or pasted tokens.
//!
//! 1. The CLI calls `connect_start` with the coding agents the user picked on that computer and
//!    shows a short user code plus a link to `/connect?code=…`.
//! 2. A signed-in human opens the link, reviews one agent per coding agent (or reconnects existing
//!    ones) and approves. The agents are created, put into an agent group and a channel, and
//!    their tokens are kept sealed (AES-GCM) until the CLI collects them.
//! 3. The CLI polls with its secret device code and receives the tokens exactly once.

use ac_core::mention::validate_handle;
use ac_core::model::{Agent, AgentOrigin, Conversation, ConversationKind, Member, Visibility};
use ac_core::{AppError, AppResult};
use ac_store::AgentGroup;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::{hash_token, new_token};
use crate::external::CreateExternalAgentInput;
use crate::{Service, require_human};

const DEVICE_PREFIX: &str = "ccd_";
const TTL_MINUTES: i64 = 15;
/// No 0/O/1/I, so codes are easy to read and type.
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const MAX_AGENTS: usize = 12;

#[derive(Debug, Clone, Serialize)]
pub struct ConnectStart {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: i64,
    pub interval: u64,
}

/// One agent handed to the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuedAgent {
    /// The coding agent it belongs to on that computer (`claude-code`, `codex`, …).
    pub platform: Option<String>,
    pub agent_id: Uuid,
    pub handle: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectPoll {
    /// pending | approved | denied | expired
    pub status: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<IssuedAgent>,
    /// The first agent's token and handle (older CLIs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectRequestView {
    pub user_code: String,
    pub client: Value,
    /// pending | approved | denied | consumed | expired
    pub status: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ApproveAgent {
    #[serde(default)]
    pub platform: Option<String>,
    /// Reconnect one of your existing external agents (its old token stops working).
    #[serde(default)]
    pub agent_id: Option<Uuid>,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub expertise: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ApproveInput {
    /// One entry per agent. Empty → the single agent described by the flattened fields.
    #[serde(default)]
    pub agents: Vec<ApproveAgent>,
    #[serde(flatten)]
    pub single: ApproveAgent,
    /// Put the agents into this agent group (created if missing).
    #[serde(default)]
    pub group_name: Option<String>,
    /// Put the agents and the approver into this channel (created as a private channel if missing).
    #[serde(default)]
    pub channel_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApproveResult {
    pub agents: Vec<Agent>,
    pub group: Option<AgentGroup>,
    pub channel: Option<Conversation>,
    /// Things that could not be done (the approval itself succeeded).
    pub warnings: Vec<String>,
}

fn user_code() -> String {
    let hex = new_token();
    let chars: String = (0..8)
        .map(|i| {
            let b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
            ALPHABET[(b as usize) % ALPHABET.len()] as char
        })
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// "abcd efgh", "ABCDEFGH", "abcd-efgh" → "ABCD-EFGH".
pub fn normalize_user_code(s: &str) -> String {
    let c: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    if c.len() == 8 { format!("{}-{}", &c[..4], &c[4..]) } else { c }
}

fn clean(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

impl Service {
    /// Public. `client` is informational (hostname, OS, coding agents) and shown to the approver.
    pub async fn connect_start(&self, client: Value) -> AppResult<ConnectStart> {
        let client = match client {
            Value::Object(o) if serde_json::to_string(&o).map(|s| s.len()).unwrap_or(0) <= 8192 => Value::Object(o),
            _ => Value::Object(Default::default()),
        };
        let device_code = format!("{DEVICE_PREFIX}{}", new_token());
        let mut last = None;
        for _ in 0..5 {
            let code = user_code();
            match self
                .inner
                .store
                .create_connect_request(
                    &hash_token(&device_code),
                    &code,
                    &client,
                    Utc::now() + Duration::minutes(TTL_MINUTES),
                )
                .await
            {
                Ok(_) => {
                    return Ok(ConnectStart {
                        device_code,
                        user_code: code,
                        expires_in: TTL_MINUTES * 60,
                        interval: 2,
                    });
                }
                // A user-code collision: try another code.
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| AppError::Internal("無法產生連線代碼".into())))
    }

    /// Public. Returns the tokens once, after approval.
    pub async fn connect_poll(&self, device_code: &str) -> AppResult<ConnectPoll> {
        let device_code = device_code.trim();
        if !device_code.starts_with(DEVICE_PREFIX) {
            return Err(AppError::Unauthorized);
        }
        let req = self
            .inner
            .store
            .connect_request_by_device(&hash_token(device_code))
            .await?
            .ok_or(AppError::Unauthorized)?;
        let status = |s: &str| ConnectPoll { status: s.into(), agents: vec![], token: None, handle: None };
        match req.status.as_str() {
            "pending" if req.expires_at <= Utc::now() => Ok(status("expired")),
            "pending" => Ok(status("pending")),
            "denied" => Ok(status("denied")),
            "approved" => {
                let sealed = self.inner.store.consume_connect_request(req.id).await?.ok_or(AppError::Unauthorized)?;
                let plain = self.secrets().decrypt(&sealed).map_err(AppError::Internal)?;
                let agents: Vec<IssuedAgent> = serde_json::from_str(&plain).map_err(AppError::internal)?;
                Ok(ConnectPoll {
                    status: "approved".into(),
                    token: agents.first().map(|a| a.token.clone()),
                    handle: agents.first().map(|a| a.handle.clone()),
                    agents,
                })
            }
            // Already collected: the device code is single-use.
            _ => Err(AppError::Unauthorized),
        }
    }

    pub async fn connect_request(&self, actor: &Member, code: &str) -> AppResult<ConnectRequestView> {
        require_human(actor)?;
        let req = self
            .inner
            .store
            .connect_request_by_user_code(&normalize_user_code(code))
            .await?
            .ok_or_else(|| AppError::not_found("找不到這個連線代碼"))?;
        let status = if req.status == "pending" && req.expires_at <= Utc::now() {
            "expired".to_string()
        } else {
            req.status.clone()
        };
        Ok(ConnectRequestView { user_code: req.user_code, client: req.client, status, expires_at: req.expires_at })
    }

    async fn pending_connect_request(&self, code: &str) -> AppResult<ac_store::ConnectRequest> {
        let req = self
            .inner
            .store
            .connect_request_by_user_code(&normalize_user_code(code))
            .await?
            .ok_or_else(|| AppError::not_found("找不到這個連線代碼"))?;
        if req.status != "pending" || req.expires_at <= Utc::now() {
            return Err(AppError::bad_request("這個連線代碼已處理過或已過期，請在終端機重新執行 crewconvo setup"));
        }
        Ok(req)
    }

    /// Approve: for each entry create a new external agent or reconnect an existing one; then put
    /// them into the group and the channel, if given. New agents are in no other channel.
    pub async fn connect_approve(&self, actor: &Member, code: &str, i: ApproveInput) -> AppResult<ApproveResult> {
        require_human(actor)?;
        let req = self.pending_connect_request(code).await?;
        let entries = if i.agents.is_empty() { vec![i.single.clone()] } else { i.agents.clone() };
        if entries.len() > MAX_AGENTS {
            return Err(AppError::bad_request(format!("一次最多連接 {MAX_AGENTS} 個 Agent")));
        }

        // Check everything before creating anything, so a mistake leaves nothing half-made.
        let mut seen: Vec<String> = Vec::new();
        for e in &entries {
            match e.agent_id {
                Some(id) => {
                    let agent = self.get_agent(actor, id).await?;
                    if agent.origin != AgentOrigin::External || agent.provider_connection_id.is_some() {
                        return Err(AppError::bad_request("只能重新連線自己接入的外部 Agent"));
                    }
                    if agent.owner_id != actor.id && !actor.is_admin {
                        return Err(AppError::forbidden("只有擁有者或管理員可以重新連線這個 Agent"));
                    }
                }
                None => {
                    let (Some(handle), Some(_)) = (clean(&e.handle), clean(&e.description)) else {
                        return Err(AppError::bad_request("每個 Agent 都要填名稱（handle）與說明"));
                    };
                    validate_handle(handle).map_err(AppError::BadRequest)?;
                    let lower = handle.to_lowercase();
                    if seen.contains(&lower)
                        || self.inner.store.member_by_handle(actor.workspace_id, handle).await?.is_some()
                    {
                        return Err(AppError::Conflict(format!("@{handle} 已經有人使用，請換一個名稱")));
                    }
                    seen.push(lower);
                }
            }
        }

        let mut agents: Vec<Agent> = Vec::new();
        let mut issued: Vec<IssuedAgent> = Vec::new();
        for e in &entries {
            let creds = match e.agent_id {
                Some(id) => self.rotate_agent_token(actor, id).await?,
                None => {
                    self.create_external_agent(
                        actor,
                        CreateExternalAgentInput {
                            handle: clean(&e.handle).unwrap_or_default().to_string(),
                            display_name: clean(&e.display_name).map(str::to_string),
                            description: clean(&e.description).unwrap_or_default().to_string(),
                            expertise: e.expertise.clone(),
                            delivery: Some("websocket".into()),
                            webhook_url: None,
                            max_wake_seconds: Some(1800),
                            daily_token_budget: None,
                            join_conversations: vec![],
                        },
                    )
                    .await?
                }
            };
            issued.push(IssuedAgent {
                platform: clean(&e.platform).map(str::to_string),
                agent_id: creds.agent.member.id,
                handle: creds.agent.member.handle.clone(),
                token: creds.token.ok_or_else(|| AppError::Internal("沒有產生 token".into()))?,
            });
            agents.push(creds.agent);
        }

        let ids: Vec<Uuid> = agents.iter().map(|a| a.member.id).collect();
        let mut warnings = Vec::new();
        let group = match clean(&i.group_name) {
            Some(name) => match self.connect_group(actor, name, &ids).await {
                Ok(g) => Some(g),
                Err(e) => {
                    warnings.push(format!("沒有建立群組「{name}」：{e}"));
                    None
                }
            },
            None => None,
        };
        let channel = match clean(&i.channel_name) {
            Some(name) => match self.connect_channel(actor, name, &ids).await {
                Ok(c) => Some(c),
                Err(e) => {
                    warnings.push(format!("沒有建立頻道「{name}」：{e}"));
                    None
                }
            },
            None => None,
        };

        let sealed = self.secrets().encrypt(&json!(issued).to_string());
        if !self
            .inner
            .store
            .decide_connect_request(req.id, true, actor.workspace_id, ids.first().copied(), Some(&sealed), actor.id)
            .await?
        {
            return Err(AppError::bad_request("這個連線代碼剛剛已處理過或已過期"));
        }
        Ok(ApproveResult { agents, group, channel, warnings })
    }

    /// Adds the agents to the group with this name (one the approver may edit), or creates it.
    async fn connect_group(&self, actor: &Member, name: &str, ids: &[Uuid]) -> AppResult<AgentGroup> {
        let store = &self.inner.store;
        let existing = store.list_agent_groups(actor.workspace_id).await?.into_iter().find(|g| g.name == name);
        let id = match existing {
            Some(g) if g.created_by == actor.id || actor.is_admin => {
                let mut members = g.agent_ids.clone();
                members.extend(ids.iter().copied().filter(|id| !g.agent_ids.contains(id)));
                store.set_agent_group_members(g.id, &members, actor.id).await?;
                g.id
            }
            Some(_) => return Err(AppError::forbidden("已經有同名的群組，而且不是你建立的")),
            None => {
                let id =
                    store.create_agent_group(actor.workspace_id, name, "由 crewconvo setup 建立", actor.id).await?;
                store.set_agent_group_members(id, ids, actor.id).await?;
                id
            }
        };
        store.get_agent_group(id).await?.ok_or_else(|| AppError::not_found("群組"))
    }

    /// Invites the agents into the approver's channel with this name, or creates it (private).
    async fn connect_channel(&self, actor: &Member, name: &str, ids: &[Uuid]) -> AppResult<Conversation> {
        let name = name.trim().trim_start_matches('#');
        let mine = self.inner.store.list_conversations_for(actor.id).await?;
        let existing = mine
            .into_iter()
            .map(|c| c.conversation)
            .find(|c| c.kind == ConversationKind::Channel && c.name.as_deref() == Some(name));
        match existing {
            Some(c) => {
                self.invite(actor, c.id, ids).await?;
                Ok(c)
            }
            None => self.create_channel(actor, name, Some("這台電腦上的 Coding Agent"), Visibility::Private, ids).await,
        }
    }

    pub async fn connect_deny(&self, actor: &Member, code: &str) -> AppResult<()> {
        require_human(actor)?;
        let req = self.pending_connect_request(code).await?;
        self.inner.store.decide_connect_request(req.id, false, actor.workspace_id, None, None, actor.id).await?;
        Ok(())
    }
}
