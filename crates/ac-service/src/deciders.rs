//! Decider configuration management (spec 6.2, 10). Admin only. Keys are write-only:
//! they are encrypted at rest and only their last 4 characters are ever returned.

use ac_core::model::{
    AgentOrigin, AgentProfile, AgentStatus, DeciderConfig, DeciderProtocol, Member, MemberKind, WakeMode,
};
use ac_core::{AppError, AppResult};
use ac_decider::secret::last4;
use ac_decider::{DecideConversation, DecideInput, DecideMessage, defaults, net, resolve};
use ac_store::DeciderConfigWrite;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::Service;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeciderConfigInput {
    pub name: Option<String>,
    pub protocol: Option<DeciderProtocol>,
    pub endpoint_url: Option<String>,
    /// New key (write-only). Omit to keep the current one.
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
    pub auth_header: Option<String>,
    pub auth_prefix: Option<String>,
    pub model: Option<String>,
    pub input_field: Option<String>,
    pub timeout_ms: Option<i32>,
    pub t_wake: Option<f32>,
    pub t_discuss: Option<f32>,
    pub max_wake: Option<i32>,
    pub extra_headers: Option<Value>,
    pub extra_body: Option<Value>,
    pub enabled: Option<bool>,
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DeciderStats {
    pub calls: i64,
    pub failures: i64,
    pub failure_rate: f64,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeciderConfigView {
    #[serde(flatten)]
    pub config: DeciderConfig,
    pub stats: DeciderStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeciderTestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
    pub error_kind: Option<String>,
    pub message: String,
    pub scores: Vec<(String, f32)>,
    pub interaction: Option<String>,
    pub interaction_confidence: Option<f32>,
    pub woken: Vec<String>,
    pub mode: Option<WakeMode>,
    pub response: Option<Value>,
}

const SAMPLE_MESSAGE: &str = "我們昨晚部署新版後 API 延遲變高了，可以幫忙看一下資料庫有沒有慢查詢嗎？";

fn default_auth(protocol: DeciderProtocol) -> (&'static str, &'static str) {
    match protocol {
        DeciderProtocol::DecisionsApi => ("x-goog-api-key", ""),
        DeciderProtocol::LlmStructured => ("Authorization", "Bearer "),
    }
}

impl Service {
    async fn validate_decider_input(&self, i: &DeciderConfigInput) -> AppResult<()> {
        if let Some(n) = &i.name
            && (n.trim().is_empty() || n.chars().count() > 80)
        {
            return Err(AppError::bad_request("name 需為 1–80 個字元"));
        }
        if let Some(u) = &i.endpoint_url {
            net::check_url(u, self.egress()).await.map_err(AppError::BadRequest)?;
        }
        if let Some(t) = i.t_wake
            && !(0.0..=0.99).contains(&t)
        {
            return Err(AppError::bad_request("t_wake 需介於 0 與 0.99（Decisions API 的分數上限是 0.99）"));
        }
        if let Some(t) = i.t_discuss
            && !(0.0..=1.0).contains(&t)
        {
            return Err(AppError::bad_request("t_discuss 需介於 0 與 1"));
        }
        if let Some(m) = i.max_wake
            && !(1..=16).contains(&m)
        {
            return Err(AppError::bad_request("max_wake 需介於 1 與 16"));
        }
        if let Some(t) = i.timeout_ms
            && !(100..=60_000).contains(&t)
        {
            return Err(AppError::bad_request("timeout_ms 需介於 100 與 60000"));
        }
        for (f, v) in [("extra_headers", &i.extra_headers), ("extra_body", &i.extra_body)] {
            if let Some(v) = v
                && !v.is_object()
            {
                return Err(AppError::bad_request(format!("{f} 必須是 JSON 物件")));
            }
        }
        if let Some(h) = &i.auth_header
            && reqwest_header_ok(h).is_err()
        {
            return Err(AppError::bad_request("auth_header 不是有效的標頭名稱"));
        }
        if let Some(f) = &i.input_field
            && (f.trim().is_empty() || !f.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            return Err(AppError::bad_request("input_field 只能包含英數字與底線"));
        }
        Ok(())
    }

    fn to_write(&self, i: &DeciderConfigInput) -> DeciderConfigWrite {
        let api_key = if i.clear_api_key {
            Some(None)
        } else {
            i.api_key
                .as_ref()
                .map(|k| k.trim())
                .filter(|k| !k.is_empty())
                .map(|k| Some((self.secrets().encrypt(k), last4(k))))
        };
        DeciderConfigWrite {
            name: i.name.as_ref().map(|s| s.trim().to_string()),
            protocol: i.protocol,
            endpoint_url: i.endpoint_url.as_ref().map(|s| s.trim().to_string()),
            api_key,
            auth_header: i.auth_header.clone(),
            auth_prefix: i.auth_prefix.clone(),
            model: i.model.as_ref().map(|s| s.trim().to_string()),
            input_field: i.input_field.clone(),
            timeout_ms: i.timeout_ms,
            t_wake: i.t_wake,
            t_discuss: i.t_discuss,
            max_wake: i.max_wake,
            extra_headers: i.extra_headers.clone(),
            extra_body: i.extra_body.clone(),
            enabled: i.enabled,
            priority: i.priority,
        }
    }

    pub async fn list_decider_configs(&self, actor: &Member) -> AppResult<Vec<DeciderConfigView>> {
        Service::require_admin(actor)?;
        let configs = self.inner.store.list_decider_configs(actor.workspace_id).await?;
        let stats = self.inner.store.decider_stats(actor.workspace_id, 100).await?;
        Ok(configs
            .into_iter()
            .map(|r| {
                let s = stats.iter().find(|s| s.0 == r.config.id);
                DeciderConfigView {
                    stats: s
                        .map(|(_, n, f, avg, p95)| DeciderStats {
                            calls: *n,
                            failures: *f,
                            failure_rate: if *n > 0 { *f as f64 / *n as f64 } else { 0.0 },
                            avg_latency_ms: *avg,
                            p95_latency_ms: *p95,
                        })
                        .unwrap_or_default(),
                    config: r.config,
                }
            })
            .collect())
    }

    pub async fn create_decider_config(&self, actor: &Member, mut i: DeciderConfigInput) -> AppResult<DeciderConfig> {
        Service::require_admin(actor)?;
        let protocol = i.protocol.ok_or_else(|| AppError::bad_request("protocol 必填"))?;
        let (h, p) = default_auth(protocol);
        if i.auth_header.as_deref().map(str::trim).unwrap_or("").is_empty() {
            i.auth_header = Some(h.into());
        }
        if i.auth_prefix.is_none() {
            i.auth_prefix = Some(p.into());
        }
        if i.name.is_none() || i.endpoint_url.is_none() || i.model.is_none() {
            return Err(AppError::bad_request("name、endpoint_url、model 必填"));
        }
        self.validate_decider_input(&i).await?;
        let rec =
            self.inner.store.create_decider_config(actor.workspace_id, &self.to_write(&i), Some(actor.id)).await?;
        Ok(rec.config)
    }

    async fn own_config(&self, actor: &Member, id: Uuid) -> AppResult<ac_store::DeciderConfigRecord> {
        let rec =
            self.inner.store.get_decider_config(id).await?.ok_or_else(|| AppError::not_found("decider config"))?;
        if rec.config.workspace_id != actor.workspace_id {
            return Err(AppError::not_found("decider config"));
        }
        Ok(rec)
    }

    pub async fn update_decider_config(
        &self,
        actor: &Member,
        id: Uuid,
        i: DeciderConfigInput,
    ) -> AppResult<DeciderConfig> {
        Service::require_admin(actor)?;
        self.own_config(actor, id).await?;
        self.validate_decider_input(&i).await?;
        let rec = self.inner.store.update_decider_config(id, &self.to_write(&i), Some(actor.id)).await?;
        Ok(rec.config)
    }

    pub async fn delete_decider_config(&self, actor: &Member, id: Uuid) -> AppResult<()> {
        Service::require_admin(actor)?;
        self.own_config(actor, id).await?;
        self.inner.store.delete_decider_config(id).await?;
        Ok(())
    }

    /// Sets priorities from the given order.
    pub async fn reorder_decider_configs(&self, actor: &Member, ids: &[Uuid]) -> AppResult<Vec<DeciderConfig>> {
        Service::require_admin(actor)?;
        for (i, id) in ids.iter().enumerate() {
            self.own_config(actor, *id).await?;
            self.inner
                .store
                .update_decider_config(
                    *id,
                    &DeciderConfigWrite { priority: Some(i as i32), ..Default::default() },
                    Some(actor.id),
                )
                .await?;
        }
        Ok(self.inner.store.list_decider_configs(actor.workspace_id).await?.into_iter().map(|r| r.config).collect())
    }

    /// Calls one config once with a built-in sample message ("test" button).
    pub async fn test_decider_config(&self, actor: &Member, id: Uuid) -> AppResult<DeciderTestResult> {
        Service::require_admin(actor)?;
        let rec = self.own_config(actor, id).await?;
        let mut candidates: Vec<AgentProfile> = self
            .inner
            .store
            .list_agents(actor.workspace_id, None, None)
            .await?
            .iter()
            .filter(|a| a.member.deactivated_at.is_none())
            .take(6)
            .map(AgentProfile::from)
            .collect();
        if candidates.is_empty() {
            let mk = |n: u128, h: &str, d: &str, e: &[&str]| AgentProfile {
                id: Uuid::from_u128(n),
                handle: h.into(),
                display_name: h.into(),
                origin: AgentOrigin::Builtin,
                description: d.into(),
                expertise: e.iter().map(|s| s.to_string()).collect(),
                status: AgentStatus::Online,
            };
            candidates = vec![
                mk(1, "dba", "資料庫效能調校與查詢優化", &["PostgreSQL", "慢查詢", "索引"]),
                mk(2, "designer", "介面與視覺設計", &["Figma", "UI"]),
                mk(3, "sre", "部署、監控與事故處理", &["Kubernetes", "延遲", "監控"]),
            ];
        }
        let input = DecideInput {
            message: DecideMessage {
                author_handle: actor.handle.clone(),
                author_kind: MemberKind::Human,
                body: SAMPLE_MESSAGE.into(),
            },
            recent: vec![],
            conversation: DecideConversation { name: Some("ops".into()), topic: Some("維運".into()), is_dm: false },
            candidates: candidates.clone(),
        };
        let handle = |id: Uuid| candidates.iter().find(|c| c.id == id).map(|c| c.handle.clone()).unwrap_or_default();
        let started = std::time::Instant::now();
        match self.inner.decider.call_one(&rec, &input).await {
            Ok(call) => {
                let (woken, mode) = resolve(&call.decision, &rec.config);
                Ok(DeciderTestResult {
                    ok: true,
                    latency_ms: call.latency_ms,
                    error: None,
                    error_kind: None,
                    message: SAMPLE_MESSAGE.into(),
                    scores: call.decision.scores.iter().map(|(id, s)| (handle(*id), *s)).collect(),
                    interaction: Some(call.decision.interaction.as_str().into()),
                    interaction_confidence: Some(call.decision.interaction_confidence),
                    woken: woken.iter().map(|(id, _)| handle(*id)).collect(),
                    mode,
                    response: Some(call.response),
                })
            }
            Err(e) => Ok(DeciderTestResult {
                ok: false,
                latency_ms: started.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
                error_kind: Some(e.kind().into()),
                message: SAMPLE_MESSAGE.into(),
                scores: vec![],
                interaction: None,
                interaction_confidence: None,
                woken: vec![],
                mode: None,
                response: None,
            }),
        }
    }

    /// Default list for a new workspace: Decisions API first (key from `GEMINI_API_KEY` if the
    /// server has one, otherwise waiting for the admin), then an OpenAI structured-output fallback
    /// when `OPENAI_API_KEY` is configured.
    pub async fn seed_default_deciders(&self, workspace_id: Uuid) -> AppResult<()> {
        if !self.inner.store.list_decider_configs(workspace_id).await?.is_empty() {
            return Ok(());
        }
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let t_wake = env("DECIDER_DEFAULT_T_WAKE").and_then(|v| v.parse().ok());
        let t_discuss = env("DECIDER_DEFAULT_T_DISCUSS").and_then(|v| v.parse().ok());
        let mut list = vec![(defaults::decisions_api(), env("GEMINI_API_KEY"))];
        if let Some(k) = env("OPENAI_API_KEY") {
            list.push((defaults::openai_structured(), Some(k)));
        }
        for (i, (d, key)) in list.into_iter().enumerate() {
            let w = DeciderConfigWrite {
                name: Some(d.name.into()),
                protocol: Some(d.protocol),
                endpoint_url: Some(d.endpoint_url.into()),
                api_key: key.map(|k| Some((self.secrets().encrypt(&k), last4(&k)))),
                auth_header: Some(d.auth_header.into()),
                auth_prefix: Some(d.auth_prefix.into()),
                model: Some(d.model.into()),
                input_field: Some("input".into()),
                timeout_ms: Some(if d.protocol == DeciderProtocol::DecisionsApi { 1500 } else { 8000 }),
                // Calibrated with `ac-cli decider eval` on fixtures/decider_zh.jsonl (docs/decider-eval.md).
                t_wake: t_wake.or(Some(d.t_wake)),
                t_discuss: t_discuss.or(Some(d.t_discuss)),
                max_wake: Some(4),
                extra_headers: Some(json!({})),
                extra_body: Some(d.extra_body),
                enabled: Some(true),
                priority: Some(i as i32),
            };
            self.inner.store.create_decider_config(workspace_id, &w, None).await?;
        }
        Ok(())
    }
}

fn reqwest_header_ok(h: &str) -> Result<(), ()> {
    if h.is_empty() || !h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(());
    }
    Ok(())
}
