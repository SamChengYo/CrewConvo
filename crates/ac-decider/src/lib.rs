//! Wake Decider (spec 6.2): decides which agents should look at a human message that no rule
//! matched. Configurable per workspace as an ordered list; each entry speaks one protocol.
//! The engine — not the model — turns scores into a result, with thresholds and caps enforced here.

mod decisions_api;
mod llm_structured;
pub mod net;
pub mod pick;
pub mod secret;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ac_core::model::{AgentProfile, DeciderConfig, DeciderProtocol, Interaction, MemberKind, WakeMode};
use ac_store::DeciderConfigRecord;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

pub use net::EgressPolicy;
pub use pick::PickInput;
pub use secret::SecretBox;

/// Recent messages are truncated to this many characters each.
pub const RECENT_CHARS: usize = 500;
pub const RECENT_COUNT: usize = 10;
/// Circuit-breaker duration after a 429.
pub const BREAKER: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecideMessage {
    pub author_handle: String,
    pub author_kind: MemberKind,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecideConversation {
    pub name: Option<String>,
    pub topic: Option<String>,
    pub is_dm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecideInput {
    pub message: DecideMessage,
    /// Oldest first, at most [`RECENT_COUNT`].
    pub recent: Vec<DecideMessage>,
    pub conversation: DecideConversation,
    pub candidates: Vec<AgentProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub scores: Vec<(Uuid, f32)>,
    pub interaction: Interaction,
    pub interaction_confidence: f32,
}

#[derive(Debug, Error)]
pub enum DeciderError {
    #[error("blocked by safety filter: {0}")]
    Safety(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("server error {0}: {1}")]
    Server(u16, String),
    #[error("timeout")]
    Timeout,
    #[error("network: {0}")]
    Network(String),
    #[error("invalid response: {0}")]
    Invalid(String),
    #[error("configuration: {0}")]
    Config(String),
}

impl DeciderError {
    pub fn kind(&self) -> &'static str {
        match self {
            DeciderError::Safety(_) => "safety",
            DeciderError::BadRequest(_) => "bad_request",
            DeciderError::RateLimited(_) => "rate_limited",
            DeciderError::Server(..) => "server",
            DeciderError::Timeout => "timeout",
            DeciderError::Network(_) => "network",
            DeciderError::Invalid(_) => "invalid_response",
            DeciderError::Config(_) => "config",
        }
    }
}

/// One successful protocol call. `request` has authentication removed.
#[derive(Debug, Clone)]
pub struct ProtocolCall {
    pub request: Value,
    pub response: Value,
    pub decision: Decision,
    pub latency_ms: u64,
}

#[async_trait]
pub trait WakeDecider: Send + Sync {
    async fn decide(&self, input: DecideInput) -> Result<Decision, DeciderError>;
}

/// Everything a protocol needs to make one call.
pub struct CallContext<'a> {
    pub config: &'a DeciderConfig,
    pub api_key: Option<String>,
    pub policy: EgressPolicy,
}

/// Question key for an agent: `[A-Za-z0-9_-]{1,64}`.
pub fn agent_key(id: Uuid) -> String {
    format!("wake_{}", id.simple())
}

fn clip(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// Shared context text (conversation, recent messages, message to judge).
pub fn context_text(input: &DecideInput) -> String {
    let mut s = String::new();
    if input.conversation.is_dm {
        s.push_str("對話：私訊\n");
    } else {
        s.push_str(&format!("對話：#{}", input.conversation.name.as_deref().unwrap_or("")));
        if let Some(t) = input.conversation.topic.as_deref().filter(|t| !t.is_empty()) {
            s.push_str(&format!("（主題：{t}）"));
        }
        s.push('\n');
    }
    if !input.recent.is_empty() {
        s.push_str("\n最近訊息（由舊到新）：\n");
        for m in &input.recent {
            let kind = if m.author_kind == MemberKind::Agent { "Agent" } else { "人類" };
            s.push_str(&format!("[{kind} @{}] {}\n", m.author_handle, clip(&m.body, RECENT_CHARS)));
        }
    }
    s.push_str(&format!(
        "\n待判斷訊息：\n[人類 @{}] {}\n",
        input.message.author_handle,
        clip(&input.message.body, 4000)
    ));
    s
}

pub fn agent_instructions(a: &AgentProfile) -> String {
    let expertise = if a.expertise.is_empty() { "（未列）".to_string() } else { a.expertise.join("、") };
    format!(
        "以下 Agent 是否應該回應最後一則訊息？Agent：{}（@{}）。職責：{}。專長：{}。",
        a.display_name, a.handle, a.description, expertise
    )
}

pub const TRUE_DESCRIPTION: &str = "訊息明確屬於此 Agent 的職責，或需要它的專長。";
pub const FALSE_DESCRIPTION: &str = "訊息與此 Agent 無關、是閒聊、或其他成員更適合處理。";
pub const INTERACTION_INSTRUCTIONS: &str = "若有多個 Agent 回應這則訊息，它們應該如何互動？";
pub const DISCUSS_DESCRIPTION: &str = "需要彼此交換意見、比較觀點才能得出結論。";
pub const INDEPENDENT_DESCRIPTION: &str = "各自回答自己負責的部分即可，不需要互相參考。";

/// Merges `extra` (a JSON object) into `base`, overriding keys except the protected ones.
pub(crate) fn merge_object(base: &mut Value, extra: &Value, protected: &[&str]) {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            if !protected.contains(&k.as_str()) {
                b.insert(k.clone(), v.clone());
            }
        }
    }
}

pub(crate) fn headers(cx: &CallContext<'_>) -> Result<reqwest::header::HeaderMap, DeciderError> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let mut h = HeaderMap::new();
    if let Some(obj) = cx.config.extra_headers.as_object() {
        for (k, v) in obj {
            let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| DeciderError::Config(e.to_string()))?;
            let val = HeaderValue::from_str(v.as_str().unwrap_or(&v.to_string()))
                .map_err(|e| DeciderError::Config(e.to_string()))?;
            h.insert(name, val);
        }
    }
    if let Some(key) = &cx.api_key {
        let name = HeaderName::from_bytes(cx.config.auth_header.as_bytes())
            .map_err(|e| DeciderError::Config(e.to_string()))?;
        let mut val = HeaderValue::from_str(&format!("{}{}", cx.config.auth_prefix, key))
            .map_err(|e| DeciderError::Config(e.to_string()))?;
        val.set_sensitive(true);
        h.insert(name, val);
    }
    Ok(h)
}

/// POSTs JSON to the configured endpoint with SSRF checks; returns (status, body json).
pub(crate) async fn post_json(cx: &CallContext<'_>, body: &Value) -> Result<(u16, Value), DeciderError> {
    let timeout = Duration::from_millis(cx.config.timeout_ms.clamp(100, 60_000) as u64);
    let (url, addrs) = net::check_url(&cx.config.endpoint_url, cx.policy).await.map_err(DeciderError::Config)?;
    let client = net::pinned_client(&url, &addrs, timeout).map_err(DeciderError::Config)?;
    let resp = client
        .post(url)
        .headers(headers(cx)?)
        .json(body)
        .send()
        .await
        .map_err(|e| if e.is_timeout() { DeciderError::Timeout } else { DeciderError::Network(e.to_string()) })?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| if e.is_timeout() { DeciderError::Timeout } else { DeciderError::Network(e.to_string()) })?;
    let v: Value =
        serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text.chars().take(2000).collect::<String>() }));
    Ok((status, v))
}

/// Calls one config.
pub async fn call(cx: &CallContext<'_>, input: &DecideInput) -> Result<ProtocolCall, DeciderError> {
    if input.candidates.is_empty() {
        return Err(DeciderError::BadRequest("no candidates".into()));
    }
    let started = Instant::now();
    let (request, response, decision) = match cx.config.protocol {
        DeciderProtocol::DecisionsApi => decisions_api::call(cx, input).await?,
        DeciderProtocol::LlmStructured => llm_structured::call(cx, input).await?,
    };
    Ok(ProtocolCall { request, response, decision, latency_ms: started.elapsed().as_millis() as u64 })
}

/// Turns raw scores into who to wake and how (the engine's rule, not the model's).
pub fn resolve(decision: &Decision, cfg: &DeciderConfig) -> (Vec<(Uuid, f32)>, Option<WakeMode>) {
    let t_wake = cfg.t_wake.clamp(0.0, 0.99);
    let t_discuss = cfg.t_discuss.clamp(0.0, 1.0);
    let max_wake = cfg.max_wake.clamp(1, 16) as usize;
    let mut woken: Vec<(Uuid, f32)> = decision.scores.iter().copied().filter(|(_, s)| *s >= t_wake).collect();
    woken.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    woken.truncate(max_wake);
    let mode = match woken.len() {
        0 => None,
        1 => Some(WakeMode::Single),
        _ if decision.interaction == Interaction::Discuss && decision.interaction_confidence >= t_discuss => {
            Some(WakeMode::Discussion)
        }
        _ => Some(WakeMode::Parallel),
    };
    (woken, mode)
}

#[derive(Debug, Clone, Serialize)]
pub struct Attempt {
    pub config_id: Uuid,
    pub name: String,
    pub ok: bool,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

pub struct ChainResult {
    pub used: Option<(DeciderConfig, ProtocolCall)>,
    pub attempts: Vec<Attempt>,
}

/// Ordered fallback over a workspace's configs, with a short circuit breaker on 429.
pub struct DeciderChain {
    secrets: SecretBox,
    policy: EgressPolicy,
    breaker: Mutex<HashMap<Uuid, Instant>>,
}

impl DeciderChain {
    pub fn new(secrets: SecretBox, policy: EgressPolicy) -> Self {
        DeciderChain { secrets, policy, breaker: Mutex::new(HashMap::new()) }
    }

    pub fn secrets(&self) -> &SecretBox {
        &self.secrets
    }
    pub fn policy(&self) -> EgressPolicy {
        self.policy
    }

    fn key_of(&self, rec: &DeciderConfigRecord) -> Result<Option<String>, DeciderError> {
        rec.api_key_encrypted.as_deref().map(|e| self.secrets.decrypt(e).map_err(DeciderError::Config)).transpose()
    }

    /// Whether a config can be called at all: enabled, and with a key where the protocol needs one.
    pub fn usable(rec: &DeciderConfigRecord) -> bool {
        rec.config.enabled
            && !(rec.config.protocol == DeciderProtocol::DecisionsApi && rec.api_key_encrypted.is_none())
    }

    /// Asks the first usable config a one-of-N question, once. None when there is no usable
    /// config or the call fails — callers fall back to a rule of their own.
    pub async fn pick(&self, configs: &[DeciderConfigRecord], input: &PickInput, cap: Duration) -> Option<String> {
        let now = Instant::now();
        let rec = {
            let breaker = self.breaker.lock().unwrap();
            let mut ordered: Vec<&DeciderConfigRecord> = configs.iter().filter(|c| Self::usable(c)).collect();
            ordered.sort_by_key(|c| c.config.priority);
            ordered.into_iter().find(|c| breaker.get(&c.config.id).is_none_or(|until| now >= *until))?
        };
        let cx = CallContext { config: &rec.config, api_key: self.key_of(rec).ok()?, policy: self.policy };
        match pick::call(&cx, input, cap).await {
            Ok(key) => Some(key),
            Err(err) => {
                if matches!(err, DeciderError::RateLimited(_)) {
                    self.breaker.lock().unwrap().insert(rec.config.id, Instant::now() + BREAKER);
                }
                tracing::warn!(config = %rec.config.name, error = %err, "decider pick failed");
                None
            }
        }
    }

    /// Calls a single config regardless of order / breaker (test button, evaluation).
    pub async fn call_one(&self, rec: &DeciderConfigRecord, input: &DecideInput) -> Result<ProtocolCall, DeciderError> {
        let cx = CallContext { config: &rec.config, api_key: self.key_of(rec)?, policy: self.policy };
        call(&cx, input).await
    }

    pub async fn decide(&self, configs: &[DeciderConfigRecord], input: &DecideInput) -> ChainResult {
        let mut attempts = Vec::new();
        let mut ordered: Vec<&DeciderConfigRecord> = configs.iter().filter(|c| c.config.enabled).collect();
        ordered.sort_by_key(|c| c.config.priority);
        for rec in ordered {
            let cfg = &rec.config;
            let skip = |why: &str| Attempt {
                config_id: cfg.id,
                name: cfg.name.clone(),
                ok: false,
                latency_ms: 0,
                error: None,
                error_kind: None,
                skipped: Some(why.into()),
            };
            if cfg.protocol == DeciderProtocol::DecisionsApi && rec.api_key_encrypted.is_none() {
                attempts.push(skip("尚未設定金鑰"));
                continue;
            }
            let open_until = self.breaker.lock().unwrap().get(&cfg.id).copied();
            if let Some(until) = open_until
                && Instant::now() < until
            {
                attempts.push(skip("熔斷中（最近收到 429）"));
                continue;
            }
            let started = Instant::now();
            match self.call_one(rec, input).await {
                Ok(call) => {
                    attempts.push(Attempt {
                        config_id: cfg.id,
                        name: cfg.name.clone(),
                        ok: true,
                        latency_ms: call.latency_ms,
                        error: None,
                        error_kind: None,
                        skipped: None,
                    });
                    return ChainResult { used: Some((cfg.clone(), call)), attempts };
                }
                Err(err) => {
                    if matches!(err, DeciderError::RateLimited(_)) {
                        self.breaker.lock().unwrap().insert(cfg.id, Instant::now() + BREAKER);
                    }
                    tracing::warn!(config = %cfg.name, error = %err, "decider attempt failed");
                    attempts.push(Attempt {
                        config_id: cfg.id,
                        name: cfg.name.clone(),
                        ok: false,
                        latency_ms: started.elapsed().as_millis() as u64,
                        error: Some(err.to_string()),
                        error_kind: Some(err.kind().into()),
                        skipped: None,
                    });
                }
            }
        }
        ChainResult { used: None, attempts }
    }
}

/// Default configs for a new workspace (spec: Decisions API first, awaiting a key).
pub mod defaults {
    use ac_core::model::DeciderProtocol;
    use serde_json::json;

    pub struct DefaultConfig {
        pub name: &'static str,
        pub protocol: DeciderProtocol,
        pub endpoint_url: &'static str,
        pub auth_header: &'static str,
        pub auth_prefix: &'static str,
        pub model: &'static str,
        pub extra_body: serde_json::Value,
        pub t_wake: f32,
        pub t_discuss: f32,
    }

    pub fn decisions_api() -> DefaultConfig {
        DefaultConfig {
            name: "Gemini Decisions API",
            protocol: DeciderProtocol::DecisionsApi,
            endpoint_url: "https://generativelanguage.googleapis.com/v1beta/decisions",
            auth_header: "x-goog-api-key",
            auth_prefix: "",
            model: "gemini-decision-1",
            extra_body: json!({}),
            t_wake: 0.60,
            t_discuss: 0.75,
        }
    }

    pub fn openai_structured() -> DefaultConfig {
        DefaultConfig {
            name: "OpenAI 結構化輸出（後備）",
            protocol: DeciderProtocol::LlmStructured,
            endpoint_url: "https://api.openai.com/v1/chat/completions",
            auth_header: "Authorization",
            auth_prefix: "Bearer ",
            model: "gpt-6-luna",
            extra_body: json!({"reasoning_effort": "none"}),
            t_wake: 0.80,
            t_discuss: 0.85,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn cfg(t_wake: f32, t_discuss: f32, max_wake: i32) -> DeciderConfig {
        DeciderConfig {
            id: Uuid::nil(),
            workspace_id: Uuid::nil(),
            name: "t".into(),
            protocol: DeciderProtocol::DecisionsApi,
            endpoint_url: String::new(),
            has_api_key: false,
            api_key_last4: None,
            auth_header: String::new(),
            auth_prefix: String::new(),
            model: String::new(),
            input_field: "input".into(),
            timeout_ms: 1500,
            t_wake,
            t_discuss,
            max_wake,
            extra_headers: json!({}),
            extra_body: json!({}),
            enabled: true,
            priority: 0,
            updated_by: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn resolve_rules() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        let d = Decision {
            scores: vec![(a, 0.9), (b, 0.7), (c, 0.2)],
            interaction: Interaction::Discuss,
            interaction_confidence: 0.8,
        };
        let (w, m) = resolve(&d, &cfg(0.6, 0.7, 4));
        assert_eq!(w.iter().map(|x| x.0).collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(m, Some(WakeMode::Discussion));
        let (_, m) = resolve(&d, &cfg(0.6, 0.85, 4));
        assert_eq!(m, Some(WakeMode::Parallel));
        let (w, m) = resolve(&d, &cfg(0.6, 0.7, 1));
        assert_eq!((w.len(), m), (1, Some(WakeMode::Single)));
        let (w, m) = resolve(&d, &cfg(0.95, 0.7, 4));
        assert!(w.is_empty() && m.is_none());
        // Thresholds above 0.99 are clamped: a manipulated 1.0 cannot bypass the cap.
        let d2 =
            Decision { scores: vec![(a, 0.99)], interaction: Interaction::Independent, interaction_confidence: 1.0 };
        assert_eq!(resolve(&d2, &cfg(5.0, 0.7, 4)).0.len(), 1);
    }

    #[test]
    fn keys_are_valid() {
        let k = agent_key(Uuid::now_v7());
        assert!(k.len() <= 64 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }
}
