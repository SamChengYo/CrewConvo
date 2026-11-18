//! Platform connections ("agent providers").
//!
//! A platform that hosts many agents is connected once: an admin gives a URL and an access token.
//! CrewConvo reads the catalog, creates one member per agent (the agent's name becomes its
//! handle), puts them in an agent group named after the platform's workspace, and wakes them over
//! the A2A protocol (see `ac-server/src/a2a.rs`). Nothing has to be set up by hand afterwards:
//! the catalog is re-read periodically, new agents appear, removed ones are deactivated.
//!
//! The URL may point to
//! - a **catalog**: `{"workspace": {"id", "name"}, "agents": [{"id", "card": <A2A AgentCard>} | {"id", "card_url"}]}`
//!   (A2A itself has no way to list many agents, so this one document is CrewConvo's addition), or
//! - a single **A2A Agent Card** (`/.well-known/agent-card.json`) — any standard A2A agent.

use std::time::Duration;

use ac_core::bus::RealtimeEvent;
use ac_core::mention::{handle_from_name, validate_handle};
use ac_core::model::{AgentOrigin, Member};
use ac_core::{AppError, AppResult};
use ac_decider::net;
use ac_store::{AgentPatch, NewAgent, ProviderConnection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::Service;

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectProviderInput {
    pub url: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderSyncReport {
    pub connection: ProviderConnection,
    pub created: Vec<String>,
    pub updated: usize,
    pub removed: usize,
    pub agents: usize,
}

/// One agent as read from a card.
#[derive(Debug, Clone, PartialEq)]
pub struct CardAgent {
    pub name: String,
    pub description: String,
    pub expertise: Vec<String>,
    /// JSON-RPC endpoint.
    pub url: String,
    /// A2A 0.x wire format (`message/send`, `kind` discriminators) instead of 1.0.
    pub legacy: bool,
}

/// Reads an A2A Agent Card (1.0 `supportedInterfaces`, or the 0.x top-level `url`).
pub fn parse_agent_card(card: &Value) -> Result<CardAgent, String> {
    let name = card["name"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("Agent Card 缺少 name")?;
    let mut endpoint: Option<(String, bool)> = None;
    if let Some(list) = card["supportedInterfaces"].as_array() {
        for i in list {
            let binding = i["protocolBinding"].as_str().unwrap_or("JSONRPC");
            if !binding.eq_ignore_ascii_case("JSONRPC") {
                continue;
            }
            if let Some(u) = i["url"].as_str() {
                let legacy = i["protocolVersion"].as_str().is_some_and(|v| v.starts_with("0."));
                endpoint = Some((u.to_string(), legacy));
                break;
            }
        }
        if endpoint.is_none() {
            return Err(format!("Agent「{name}」沒有 JSONRPC 介面（supportedInterfaces）"));
        }
    } else if let Some(u) = card["url"].as_str() {
        let transport = card["preferredTransport"].as_str().unwrap_or("JSONRPC");
        if !transport.eq_ignore_ascii_case("JSONRPC") {
            return Err(format!("Agent「{name}」的 preferredTransport 不是 JSONRPC"));
        }
        endpoint = Some((u.to_string(), true));
    }
    let (url, legacy) = endpoint.ok_or_else(|| format!("Agent「{name}」的 Card 沒有服務網址"))?;
    let mut expertise: Vec<String> = Vec::new();
    for s in card["skills"].as_array().into_iter().flatten() {
        for t in
            s["name"].as_str().into_iter().chain(s["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str()))
        {
            let t = t.trim();
            if !t.is_empty() && !expertise.iter().any(|e| e == t) && expertise.len() < 10 {
                expertise.push(t.chars().take(40).collect());
            }
        }
    }
    let description = card["description"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(name);
    Ok(CardAgent {
        name: name.to_string(),
        description: description.chars().take(1000).collect(),
        expertise,
        url,
        legacy,
    })
}

struct CatalogAgent {
    external_id: String,
    card: CardAgent,
}

struct Catalog {
    workspace_id: Option<String>,
    workspace_name: Option<String>,
    agents: Vec<CatalogAgent>,
}

fn str_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

impl Service {
    async fn provider_get(&self, url: &str, token: Option<&str>) -> AppResult<(Value, net::Url)> {
        let (u, addrs) = net::check_url(url, self.egress()).await.map_err(AppError::BadRequest)?;
        let client = net::pinned_client(&u, &addrs, Duration::from_secs(20)).map_err(AppError::Internal)?;
        let mut req = client.get(u.clone()).header("accept", "application/json").header("A2A-Version", "1.0");
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let res = req.send().await.map_err(|e| AppError::bad_request(format!("連不到 {url}：{e}")))?;
        let status = res.status();
        if !status.is_success() {
            let hint = if status.as_u16() == 401 || status.as_u16() == 403 {
                "（Access Token 不正確或沒有權限）"
            } else {
                ""
            };
            return Err(AppError::bad_request(format!("{url} 回應 HTTP {status}{hint}")));
        }
        let v: Value = res.json().await.map_err(|e| AppError::bad_request(format!("{url} 回的不是 JSON：{e}")))?;
        Ok((v, u))
    }

    async fn read_catalog(&self, conn: &ProviderConnection, token: Option<&str>) -> AppResult<Catalog> {
        let (doc, base) = self.provider_get(&conn.catalog_url, token).await?;
        let Some(list) = doc["agents"].as_array() else {
            // A single standard A2A Agent Card.
            let card = parse_agent_card(&doc).map_err(AppError::BadRequest)?;
            return Ok(Catalog {
                workspace_id: base.host_str().map(str::to_string),
                workspace_name: doc["provider"]["organization"].as_str().map(str::to_string),
                agents: vec![CatalogAgent { external_id: card.url.clone(), card }],
            });
        };
        if list.len() > 500 {
            return Err(AppError::bad_request("目錄裡的 Agent 超過 500 個"));
        }
        let mut agents = Vec::new();
        for entry in list {
            let inline = [&entry["card"], &entry["agentCard"]].into_iter().find(|c| c.is_object());
            let card_url = entry["card_url"].as_str().or_else(|| entry["cardUrl"].as_str());
            let card_doc = match (inline, card_url) {
                (Some(c), _) => c.clone(),
                (None, Some(u)) => {
                    let abs = base.join(u).map_err(|e| AppError::bad_request(format!("card_url 格式錯誤：{e}")))?;
                    self.provider_get(abs.as_str(), token).await?.0
                }
                // The entry itself is a card.
                (None, None) => entry.clone(),
            };
            let card = parse_agent_card(&card_doc).map_err(AppError::BadRequest)?;
            let external_id = str_of(&entry["id"]).unwrap_or_else(|| card.url.clone());
            if agents.iter().any(|a: &CatalogAgent| a.external_id == external_id) {
                return Err(AppError::bad_request(format!("目錄裡有重複的 Agent id：{external_id}")));
            }
            agents.push(CatalogAgent { external_id, card });
        }
        Ok(Catalog {
            workspace_id: str_of(&doc["workspace"]["id"]),
            workspace_name: str_of(&doc["workspace"]["name"]),
            agents,
        })
    }

    /// The agent's name as its handle. If another member already has it, the platform workspace
    /// tells them apart: `名稱-工作區名稱`, then `名稱-工作區id`.
    async fn provider_handle(&self, ws: Uuid, name: &str, cat: &Catalog) -> AppResult<String> {
        let base = handle_from_name(name).unwrap_or_else(|| "agent".to_string());
        let fit = |base: &str, suffix: &str| -> String {
            let keep = 32usize.saturating_sub(suffix.chars().count() + 1).max(1);
            let head: String = base.chars().take(keep).collect();
            format!("{}-{suffix}", head.trim_end_matches(['-', '.']))
        };
        let mut candidates = vec![base.clone()];
        if let Some(n) = cat.workspace_name.as_deref().and_then(handle_from_name) {
            candidates.push(fit(&base, &n.chars().take(14).collect::<String>()));
        }
        if let Some(id) = &cat.workspace_id {
            let short: String = id.chars().filter(|c| c.is_alphanumeric()).take(6).collect();
            if !short.is_empty() {
                candidates.push(fit(&base, &short));
            }
        }
        for n in 2..50 {
            candidates.push(fit(&base, &n.to_string()));
        }
        for c in candidates {
            if validate_handle(&c).is_ok() && self.inner.store.member_by_handle(ws, &c).await?.is_none() {
                return Ok(c);
            }
        }
        Err(AppError::Conflict(format!("找不到可用的 handle：{name}")))
    }

    pub async fn list_provider_connections(&self, actor: &Member) -> AppResult<Vec<ProviderConnection>> {
        Service::require_admin(actor)?;
        self.inner.store.list_provider_connections(actor.workspace_id).await
    }

    /// Connects a platform and syncs it at once. Fails (and stores nothing) if the first sync fails.
    pub async fn connect_provider(&self, actor: &Member, i: ConnectProviderInput) -> AppResult<ProviderSyncReport> {
        Service::require_admin(actor)?;
        let url = i.url.trim().to_string();
        let (parsed, _) = net::check_url(&url, self.egress()).await.map_err(AppError::BadRequest)?;
        let token = i.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let name = i
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| parsed.host_str().unwrap_or("platform").to_string());
        let sealed = token.map(|t| self.secrets().encrypt(t));
        let conn = self
            .inner
            .store
            .create_provider_connection(actor.workspace_id, &name, &url, sealed.as_deref(), actor.id)
            .await?;
        match self.sync_provider(conn.id).await {
            Ok(r) => Ok(r),
            Err(e) => {
                let _ = self.remove_provider(conn.id).await;
                Err(e)
            }
        }
    }

    pub async fn sync_provider_connection(&self, actor: &Member, id: Uuid) -> AppResult<ProviderSyncReport> {
        Service::require_admin(actor)?;
        self.owned_connection(actor, id).await?;
        self.sync_provider(id).await
    }

    pub async fn delete_provider_connection(&self, actor: &Member, id: Uuid) -> AppResult<()> {
        Service::require_admin(actor)?;
        self.owned_connection(actor, id).await?;
        self.remove_provider(id).await
    }

    async fn owned_connection(&self, actor: &Member, id: Uuid) -> AppResult<ProviderConnection> {
        self.inner
            .store
            .get_provider_connection(id)
            .await?
            .filter(|c| c.workspace_id == actor.workspace_id)
            .ok_or_else(|| AppError::not_found("平台連線"))
    }

    async fn remove_provider(&self, id: Uuid) -> AppResult<()> {
        let Some(conn) = self.inner.store.get_provider_connection(id).await? else { return Ok(()) };
        for a in self.inner.store.provider_agents(id).await? {
            if a.active {
                self.retire_provider_agent(a.member_id).await?;
            }
        }
        if let Some(g) = conn.group_id {
            self.inner.store.delete_agent_group(g).await?;
        }
        self.inner.store.delete_provider_connection(id).await?;
        self.inner.engine.bus().publish(RealtimeEvent::AgentsChanged { workspace_id: conn.workspace_id });
        Ok(())
    }

    async fn retire_provider_agent(&self, member_id: Uuid) -> AppResult<()> {
        self.inner.store.deactivate_agent(member_id).await?;
        self.inner.engine.disconnect_agent(member_id);
        if let Some(a) = self.inner.store.get_agent(member_id).await? {
            self.notify_agent_changed(&a);
        }
        Ok(())
    }

    /// Re-reads every connection's catalog (called periodically by the server).
    pub async fn sync_all_providers(&self) {
        let conns = match self.inner.store.all_provider_connections().await {
            Ok(c) => c,
            Err(e) => return tracing::warn!(error = %e, "could not list platform connections"),
        };
        for c in conns {
            if let Err(e) = self.sync_provider(c.id).await {
                tracing::warn!(connection = %c.name, error = %e, "platform sync failed");
            }
        }
    }

    pub async fn sync_provider(&self, id: Uuid) -> AppResult<ProviderSyncReport> {
        let conn =
            self.inner.store.get_provider_connection(id).await?.ok_or_else(|| AppError::not_found("平台連線"))?;
        match self.sync_provider_inner(&conn).await {
            Ok(r) => Ok(r),
            Err(e) => {
                let msg: String = e.to_string().chars().take(500).collect();
                self.inner.store.record_provider_sync(id, None, None, None, None, Some(&msg)).await?;
                Err(e)
            }
        }
    }

    async fn sync_provider_inner(&self, conn: &ProviderConnection) -> AppResult<ProviderSyncReport> {
        let store = &self.inner.store;
        let token = match &conn.token_sealed {
            Some(s) => Some(self.secrets().decrypt(s).map_err(AppError::Internal)?),
            None => None,
        };
        let cat = self.read_catalog(conn, token.as_deref()).await?;
        // Check every agent URL before touching anything.
        for a in &cat.agents {
            net::check_url(&a.card.url, self.egress())
                .await
                .map_err(|e| AppError::bad_request(format!("Agent「{}」的網址不可用：{e}", a.card.name)))?;
        }
        let existing = store.provider_agents(conn.id).await?;
        let (mut created, mut updated, mut removed) = (Vec::new(), 0usize, 0usize);
        let mut active: Vec<Uuid> = Vec::new();
        for a in &cat.agents {
            let member_id = match existing.iter().find(|e| e.external_id == a.external_id) {
                Some(e) => {
                    if !e.active {
                        store.reactivate_provider_agent(e.member_id).await?;
                    }
                    store
                        .update_agent(
                            e.member_id,
                            &AgentPatch {
                                display_name: Some(a.card.name.clone()),
                                description: Some(a.card.description.clone()),
                                expertise: Some(a.card.expertise.clone()),
                                ..Default::default()
                            },
                        )
                        .await?;
                    updated += 1;
                    e.member_id
                }
                None => {
                    let handle = self.provider_handle(conn.workspace_id, &a.card.name, &cat).await?;
                    let agent = store
                        .create_agent(&NewAgent {
                            workspace_id: conn.workspace_id,
                            handle: handle.clone(),
                            display_name: a.card.name.clone(),
                            origin: AgentOrigin::External,
                            owner_id: conn.created_by,
                            description: a.card.description.clone(),
                            expertise: a.card.expertise.clone(),
                            model: None,
                            system_prompt: None,
                            tools: json!([]),
                            max_wake_seconds: 600,
                            daily_token_budget: None,
                            sandbox_config: json!({}),
                        })
                        .await?;
                    created.push(handle);
                    agent.member.id
                }
            };
            store.set_provider_agent(member_id, conn.id, &a.external_id, &a.card.url, a.card.legacy).await?;
            active.push(member_id);
        }
        for e in existing.iter().filter(|e| e.active && !cat.agents.iter().any(|a| a.external_id == e.external_id)) {
            self.retire_provider_agent(e.member_id).await?;
            removed += 1;
        }

        // The agent group of this platform workspace: its agents, plus whatever was added by hand.
        let ours: Vec<Uuid> = existing.iter().map(|e| e.member_id).collect();
        let group = match conn.group_id {
            Some(g) => store.get_agent_group(g).await?,
            None => None,
        };
        let group_id = match group {
            Some(g) => {
                let mut members: Vec<Uuid> = g.agent_ids.iter().copied().filter(|id| !ours.contains(id)).collect();
                members.extend(active.iter().copied());
                store.set_agent_group_members(g.id, &members, conn.created_by).await?;
                g.id
            }
            None => {
                let base = cat.workspace_name.clone().unwrap_or_else(|| conn.name.clone());
                let taken: Vec<String> =
                    store.list_agent_groups(conn.workspace_id).await?.into_iter().map(|g| g.name).collect();
                let short = cat.workspace_id.as_deref().unwrap_or("").chars().take(8).collect::<String>();
                let name = [base.clone(), format!("{base}（{short}）")]
                    .into_iter()
                    .chain((2..50).map(|n| format!("{base} {n}")))
                    .find(|n| !taken.contains(n))
                    .unwrap_or(base);
                let desc = format!("由平台連線「{}」自動建立與同步", conn.name);
                let gid = store.create_agent_group(conn.workspace_id, &name, &desc, conn.created_by).await?;
                store.set_agent_group_members(gid, &active, conn.created_by).await?;
                gid
            }
        };
        store
            .record_provider_sync(
                conn.id,
                None,
                cat.workspace_id.as_deref(),
                cat.workspace_name.as_deref(),
                Some(group_id),
                None,
            )
            .await?;
        for id in &active {
            if let Some(a) = store.get_agent(*id).await? {
                self.notify_agent_changed(&a);
            }
        }
        if !created.is_empty() || removed > 0 || updated > 0 {
            self.inner.engine.bus().publish(RealtimeEvent::AgentsChanged { workspace_id: conn.workspace_id });
        }
        let connection =
            store.get_provider_connection(conn.id).await?.ok_or_else(|| AppError::not_found("平台連線"))?;
        Ok(ProviderSyncReport { connection, created, updated, removed, agents: active.len() })
    }

    /// Where and how to call an A2A agent, with the decrypted access token.
    pub async fn a2a_target(&self, agent_id: Uuid) -> AppResult<Option<(ac_store::A2aTarget, Option<String>)>> {
        let Some(t) = self.inner.store.a2a_target(agent_id).await? else { return Ok(None) };
        let token = match &t.token_sealed {
            Some(s) => Some(self.secrets().decrypt(s).map_err(AppError::Internal)?),
            None => None,
        };
        Ok(Some((t, token)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_v1_and_legacy_cards() {
        let v1 = json!({"name": "客服助理", "description": "回答客服問題",
            "supportedInterfaces": [
                {"url": "grpc://x", "protocolBinding": "GRPC", "protocolVersion": "1.0"},
                {"url": "https://p.example/a2a/1", "protocolBinding": "JSONRPC", "protocolVersion": "1.0"}],
            "skills": [{"id": "faq", "name": "常見問題", "description": "", "tags": ["客服", "退貨"]}]});
        let c = parse_agent_card(&v1).unwrap();
        assert_eq!((c.url.as_str(), c.legacy), ("https://p.example/a2a/1", false));
        assert_eq!(c.expertise, vec!["常見問題", "客服", "退貨"]);
        let old = json!({"name": "Old", "url": "https://p.example/old", "protocolVersion": "0.3.0"});
        let c = parse_agent_card(&old).unwrap();
        assert!(c.legacy);
        assert_eq!(c.description, "Old");
        assert!(parse_agent_card(&json!({"name": "x"})).is_err());
        assert!(
            parse_agent_card(&json!({"name": "x", "supportedInterfaces": [{"url": "u", "protocolBinding": "GRPC"}]}))
                .is_err()
        );
    }
}

impl Service {
    /// Makes the stored platform connections match the configuration (`A2A_CONNECTIONS`):
    /// listed ones are created or updated, others are removed. Does nothing until the first
    /// account exists (connections belong to an admin).
    pub async fn reconcile_providers(&self, specs: &[ConnectProviderInput]) -> AppResult<()> {
        let store = &self.inner.store;
        let Some(ws) = store.default_workspace().await? else { return Ok(()) };
        let Some(admin) = store.first_admin_id(ws.id).await? else { return Ok(()) };
        let existing = store.list_provider_connections(ws.id).await?;
        for spec in specs {
            let url = spec.url.trim();
            let token = spec.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
            let name = spec.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
            match existing.iter().find(|c| c.catalog_url == url) {
                Some(c) => {
                    let current = match &c.token_sealed {
                        Some(s) => self.secrets().decrypt(s).ok(),
                        None => None,
                    };
                    if current.as_deref() != token || name.is_some_and(|n| n != c.name) {
                        let sealed = token.map(|t| self.secrets().encrypt(t));
                        store.set_provider_token(c.id, sealed.as_deref(), name).await?;
                    }
                }
                None => {
                    if let Err(e) = net::check_url(url, self.egress()).await {
                        tracing::warn!(url, error = %e, "A2A_CONNECTIONS: skipping an unusable URL");
                        continue;
                    }
                    let sealed = token.map(|t| self.secrets().encrypt(t));
                    let fallback = url.split('/').nth(2).unwrap_or("platform").to_string();
                    let c = store
                        .create_provider_connection(ws.id, name.unwrap_or(&fallback), url, sealed.as_deref(), admin)
                        .await?;
                    tracing::info!(url, "platform connection added from configuration");
                    // A failed first sync is kept (with its error) and retried on the next tick.
                    if let Err(e) = self.sync_provider(c.id).await {
                        tracing::warn!(url, error = %e, "platform sync failed");
                    }
                }
            }
        }
        for c in existing.iter().filter(|c| !specs.iter().any(|s| s.url.trim() == c.catalog_url)) {
            tracing::info!(url = %c.catalog_url, "platform connection removed (no longer in the configuration)");
            self.remove_provider(c.id).await?;
        }
        Ok(())
    }

    /// Whether any human account exists yet (first-run setup).
    pub async fn has_accounts(&self) -> AppResult<bool> {
        let store = &self.inner.store;
        match store.default_workspace().await? {
            Some(ws) => Ok(store.count_humans(ws.id).await? > 0),
            None => Ok(false),
        }
    }
}
