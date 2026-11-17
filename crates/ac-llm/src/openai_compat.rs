//! OpenAI-compatible chat completions with function tools (vLLM, Ollama, OpenRouter, …).

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    ChatMessage, CompletionRequest, CompletionResponse, LlmError, LlmProvider, ToolCall, Usage, http_err, read_json,
};

pub struct OpenAiCompatibleProvider {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    max_tokens_field: String,
}

impl OpenAiCompatibleProvider {
    pub fn new(http: reqwest::Client, api_key: String, base_url: String, max_tokens_field: String) -> Self {
        Self { http, api_key, base_url: base_url.trim_end_matches('/').to_string(), max_tokens_field }
    }
}

pub(crate) fn to_chat_messages(system: &str, msgs: &[ChatMessage]) -> Vec<Value> {
    let mut out = vec![json!({"role":"system","content":system})];
    for m in msgs {
        match m {
            ChatMessage::User { content } => out.push(json!({"role":"user","content":content})),
            ChatMessage::Assistant { content, tool_calls, .. } => {
                let mut v = json!({"role":"assistant","content":content});
                if !tool_calls.is_empty() {
                    v["tool_calls"] = Value::from(
                        tool_calls
                            .iter()
                            .map(|c| json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments.to_string()}}))
                            .collect::<Vec<_>>(),
                    );
                }
                out.push(v);
            }
            ChatMessage::Tool { call_id, content, .. } => {
                out.push(json!({"role":"tool","tool_call_id":call_id,"content":content}))
            }
        }
    }
    out
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut body = json!({
            "model": req.model,
            "messages": to_chat_messages(&req.system, &req.messages),
        });
        body[self.max_tokens_field.as_str()] = json!(req.max_tokens);
        if !req.tools.is_empty() {
            body["tools"] = Value::from(
                req.tools
                    .iter()
                    .map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.input_schema}}))
                    .collect::<Vec<_>>(),
            );
        }
        if let Some(js) = &req.json_schema {
            body["response_format"] =
                json!({"type":"json_schema","json_schema":{"name":js.name,"schema":js.schema,"strict":true}});
        }
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(http_err)?;
        let v = read_json(resp).await?;
        let choice = &v["choices"][0];
        let msg = &choice["message"];
        let mut tool_calls = Vec::new();
        for c in msg["tool_calls"].as_array().cloned().unwrap_or_default() {
            let raw = c["function"]["arguments"].as_str().unwrap_or("{}");
            tool_calls.push(ToolCall {
                id: c["id"].as_str().unwrap_or_default().to_string(),
                name: c["function"]["name"].as_str().unwrap_or_default().to_string(),
                arguments: serde_json::from_str(raw).unwrap_or(Value::String(raw.to_string())),
            });
        }
        Ok(CompletionResponse {
            content: msg["content"].as_str().filter(|s| !s.is_empty()).map(String::from),
            tool_calls,
            usage: Usage {
                input_tokens: v["usage"]["prompt_tokens"].as_i64().unwrap_or(0),
                output_tokens: v["usage"]["completion_tokens"].as_i64().unwrap_or(0),
            },
            stop_reason: choice["finish_reason"].as_str().map(String::from),
            provider_state: None,
        })
    }
}
