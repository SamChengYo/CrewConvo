//! `crewconvo connect <server>`: browser approval instead of invite codes or pasted tokens.
//!
//! 1. The CLI calls `connect_start` and shows a short user code plus a link to `/connect?code=…`.
//! 2. A signed-in human opens the link, names the agent (or picks one of their external agents)
//!    and approves. A token is minted and kept sealed (AES-GCM) until the CLI collects it.
//! 3. The CLI polls with its secret device code and receives the token exactly once.

use ac_core::model::{Agent, AgentOrigin, Member};
use ac_core::{AppError, AppResult};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::{hash_token, new_token};
use crate::external::CreateExternalAgentInput;
use crate::{Service, require_human};

const DEVICE_PREFIX: &str = "ccd_";
const TTL_MINUTES: i64 = 15;
/// No 0/O/1/I, so codes are easy to read and type.
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Debug, Clone, Serialize)]
pub struct ConnectStart {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: i64,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectPoll {
    /// pending | approved | denied | expired
    pub status: String,
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
pub struct ApproveInput {
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

impl Service {
    /// Public. `client` is informational (hostname, OS, platforms) and shown to the approver.
    pub async fn connect_start(&self, client: Value) -> AppResult<ConnectStart> {
        let client = match client {
            Value::Object(o) if serde_json::to_string(&o).map(|s| s.len()).unwrap_or(0) <= 2048 => Value::Object(o),
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

    /// Public. Returns the token once, after approval.
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
        let status = |s: &str| ConnectPoll { status: s.into(), token: None, handle: None };
        match req.status.as_str() {
            "pending" if req.expires_at <= Utc::now() => Ok(status("expired")),
            "pending" => Ok(status("pending")),
            "denied" => Ok(status("denied")),
            "approved" => {
                let sealed = self.inner.store.consume_connect_request(req.id).await?.ok_or(AppError::Unauthorized)?;
                let token = self.secrets().decrypt(&sealed).map_err(AppError::Internal)?;
                let handle = match req.agent_id {
                    Some(id) => self.inner.store.get_member(id).await?.map(|m| m.handle),
                    None => None,
                };
                Ok(ConnectPoll { status: "approved".into(), token: Some(token), handle })
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
            return Err(AppError::bad_request("這個連線代碼已處理過或已過期，請在終端機重新執行 crewconvo connect"));
        }
        Ok(req)
    }

    /// Approve: create a new external agent (in no channel yet) or reconnect an existing one.
    pub async fn connect_approve(&self, actor: &Member, code: &str, i: ApproveInput) -> AppResult<Agent> {
        require_human(actor)?;
        let req = self.pending_connect_request(code).await?;
        let creds = match i.agent_id {
            Some(id) => {
                let agent = self.get_agent(actor, id).await?;
                if agent.origin != AgentOrigin::External {
                    return Err(AppError::bad_request("只能重新連線外部 Agent"));
                }
                self.rotate_agent_token(actor, id).await?
            }
            None => {
                let handle = i.handle.as_deref().map(str::trim).filter(|h| !h.is_empty());
                let description = i.description.as_deref().map(str::trim).filter(|d| !d.is_empty());
                let (Some(handle), Some(description)) = (handle, description) else {
                    return Err(AppError::bad_request("請填 handle 與描述"));
                };
                // A new agent starts in no channel: its owner invites it where it is needed.
                self.create_external_agent(
                    actor,
                    CreateExternalAgentInput {
                        handle: handle.to_string(),
                        display_name: i.display_name.clone().filter(|n| !n.trim().is_empty()),
                        description: description.to_string(),
                        expertise: i.expertise.clone(),
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
        let token = creds.token.ok_or_else(|| AppError::Internal("沒有產生 token".into()))?;
        let sealed = self.secrets().encrypt(&token);
        let agent_id = creds.agent.member.id;
        if !self
            .inner
            .store
            .decide_connect_request(req.id, true, actor.workspace_id, Some(agent_id), Some(&sealed), actor.id)
            .await?
        {
            return Err(AppError::bad_request("這個連線代碼剛剛已處理過或已過期"));
        }
        Ok(creds.agent)
    }

    pub async fn connect_deny(&self, actor: &Member, code: &str) -> AppResult<()> {
        require_human(actor)?;
        let req = self.pending_connect_request(code).await?;
        self.inner.store.decide_connect_request(req.id, false, actor.workspace_id, None, None, actor.id).await?;
        Ok(())
    }
}
