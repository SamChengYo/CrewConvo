//! LLM providers. Models are configured as `provider:model` strings:
//!
//! | provider        | API                                   | example                          |
//! |-----------------|---------------------------------------|----------------------------------|
//! | `anthropic`     | Anthropic Messages (native)           | `anthropic:claude-sonnet-5-5`    |
//! | `openai`        | OpenAI Responses (native)             | `openai:gpt-6-luna`              |
//! | `gemini`        | Gemini generateContent (native)       | `gemini:gemini-4-hydrogen`       |
//! | `openai_compat` | OpenAI-compatible chat completions    | `openai_compat:llama-4`          |
//! | `mock`          | deterministic script for tests/demos  | `mock:reply`                     |

mod anthropic;
mod gemini;
mod mock;
mod openai;
mod openai_compat;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ac_core::tool::ToolSpec;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use mock::{MockProvider, PAYLOAD_CLOSE, PAYLOAD_OPEN};
pub use openai::OpenAiResponsesProvider;
pub use openai_compat::OpenAiCompatibleProvider;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
        /// Provider-specific raw output that must be replayed verbatim on the next turn
        /// (OpenAI reasoning items, Gemini thought signatures, Anthropic thinking blocks).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_state: Option<Value>,
    },
    Tool {
        call_id: String,
        name: String,
        content: String,
        is_error: bool,
    },
}

/// Request JSON output matching a schema (structured output).
#[derive(Debug, Clone)]
pub struct JsonSchemaSpec {
    pub name: String,
    pub schema: Value,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    pub json_schema: Option<JsonSchemaSpec>,
}

impl CompletionRequest {
    pub fn simple(
        model: impl Into<String>,
        system: impl Into<String>,
        user: impl Into<String>,
        max_tokens: u32,
    ) -> Self {
        CompletionRequest {
            model: model.into(),
            system: system.into(),
            messages: vec![ChatMessage::User { content: user.into() }],
            tools: vec![],
            max_tokens,
            json_schema: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

impl Usage {
    pub fn total(&self) -> i64 {
        self.input_tokens + self.output_tokens
    }
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub stop_reason: Option<String>,
    pub provider_state: Option<Value>,
}

impl CompletionResponse {
    pub fn into_assistant_message(self) -> ChatMessage {
        ChatMessage::Assistant {
            content: self.content,
            tool_calls: self.tool_calls,
            provider_state: self.provider_state,
        }
    }
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("unknown provider in model string: {0}")]
    UnknownProvider(String),
    #[error("provider not configured: {0}")]
    NotConfigured(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("api error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError>;
}

/// Provider configuration from the environment.
#[derive(Debug, Clone, Default)]
pub struct LlmConfig {
    pub anthropic_api_key: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub gemini_api_key: Option<String>,
    pub gemini_base_url: Option<String>,
    pub openai_compat_api_key: Option<String>,
    pub openai_compat_base_url: Option<String>,
    pub openai_compat_max_tokens_field: Option<String>,
    pub enable_mock: bool,
}

impl LlmConfig {
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        LlmConfig {
            anthropic_api_key: get("ANTHROPIC_API_KEY"),
            anthropic_base_url: get("ANTHROPIC_BASE_URL"),
            openai_api_key: get("OPENAI_API_KEY"),
            openai_base_url: get("OPENAI_BASE_URL"),
            gemini_api_key: get("GEMINI_API_KEY"),
            gemini_base_url: get("GEMINI_BASE_URL"),
            openai_compat_api_key: get("OPENAI_COMPAT_API_KEY"),
            openai_compat_base_url: get("OPENAI_COMPAT_BASE_URL"),
            openai_compat_max_tokens_field: get("OPENAI_COMPAT_MAX_TOKENS_FIELD"),
            enable_mock: get("CREWCONVO_ENABLE_MOCK_LLM").map(|v| v != "0" && v != "false").unwrap_or(true),
        }
    }
}

pub fn http_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder().timeout(timeout).connect_timeout(Duration::from_secs(15)).build().expect("http client")
}

#[derive(Clone, Default)]
pub struct LlmRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
}

impl LlmRegistry {
    pub fn from_config(cfg: &LlmConfig) -> Self {
        let http = http_client(Duration::from_secs(300));
        let mut r = LlmRegistry::default();
        if let Some(key) = &cfg.anthropic_api_key {
            r.register(
                "anthropic",
                Arc::new(AnthropicProvider::new(
                    http.clone(),
                    key.clone(),
                    cfg.anthropic_base_url.clone().unwrap_or_else(|| "https://api.anthropic.com".into()),
                )),
            );
        }
        if let Some(key) = &cfg.openai_api_key {
            r.register(
                "openai",
                Arc::new(OpenAiResponsesProvider::new(
                    http.clone(),
                    key.clone(),
                    cfg.openai_base_url.clone().unwrap_or_else(|| "https://api.openai.com/v1".into()),
                )),
            );
        }
        if let Some(key) = &cfg.gemini_api_key {
            r.register(
                "gemini",
                Arc::new(GeminiProvider::new(
                    http.clone(),
                    key.clone(),
                    cfg.gemini_base_url
                        .clone()
                        .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".into()),
                )),
            );
        }
        if let (Some(key), Some(base)) = (&cfg.openai_compat_api_key, &cfg.openai_compat_base_url) {
            r.register(
                "openai_compat",
                Arc::new(OpenAiCompatibleProvider::new(
                    http.clone(),
                    key.clone(),
                    base.clone(),
                    cfg.openai_compat_max_tokens_field.clone().unwrap_or_else(|| "max_tokens".into()),
                )),
            );
        }
        if cfg.enable_mock {
            r.register("mock", Arc::new(MockProvider));
        }
        r
    }

    pub fn register(&mut self, name: &str, p: Arc<dyn LlmProvider>) {
        self.providers.insert(name.to_string(), p);
    }

    pub fn provider_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.providers.keys().cloned().collect();
        v.sort();
        v
    }

    /// Splits `provider:model` and returns the provider and the bare model id.
    pub fn resolve(&self, model: &str) -> Result<(Arc<dyn LlmProvider>, String), LlmError> {
        let (provider, name) = model.split_once(':').ok_or_else(|| LlmError::UnknownProvider(model.into()))?;
        let p = self.providers.get(provider).cloned().ok_or_else(|| LlmError::NotConfigured(provider.to_string()))?;
        Ok((p, name.to_string()))
    }

    /// Convenience: one completion for a `provider:model` string.
    pub async fn complete(&self, mut req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (p, model) = self.resolve(&req.model)?;
        req.model = model;
        p.complete(req).await
    }
}

/// Extracts the first JSON object from model text (tolerates code fences / prose).
pub fn extract_json(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return Some(v);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

pub(crate) fn http_err(e: reqwest::Error) -> LlmError {
    LlmError::Http(e.to_string())
}

pub(crate) async fn read_json(resp: reqwest::Response) -> Result<Value, LlmError> {
    let status = resp.status();
    let text = resp.text().await.map_err(http_err)?;
    if !status.is_success() {
        return Err(LlmError::Api { status: status.as_u16(), body: text.chars().take(2000).collect() });
    }
    serde_json::from_str(&text).map_err(|e| LlmError::InvalidResponse(e.to_string()))
}
