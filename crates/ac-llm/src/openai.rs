//! Native OpenAI Responses API (`/v1/responses`). Stateless (`store: false`): reasoning items
//! are requested with encrypted content and replayed verbatim on the next turn.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    ChatMessage, CompletionRequest, CompletionResponse, LlmError, LlmProvider, ToolCall, Usage, http_err, read_json,
};

pub struct OpenAiResponsesProvider {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl OpenAiResponsesProvider {
    pub fn new(http: reqwest::Client, api_key: String, base_url: String) -> Self {
        Self { http, api_key, base_url: base_url.trim_end_matches('/').to_string() }
    }
}

fn to_input(msgs: &[ChatMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in msgs {
        match m {
            ChatMessage::User { content } => out.push(json!({"role":"user","content":content})),
            ChatMessage::Assistant { content, tool_calls, provider_state } => {
                if let Some(Value::Array(items)) = provider_state {
                    out.extend(items.iter().cloned());
                    continue;
                }
                if let Some(t) = content.as_ref().filter(|t| !t.is_empty()) {
                    out.push(json!({"role":"assistant","content":t}));
                }
                for c in tool_calls {
                    out.push(json!({"type":"function_call","call_id":c.id,"name":c.name,"arguments":c.arguments.to_string()}));
                }
            }
            ChatMessage::Tool { call_id, content, .. } => {
                out.push(json!({"type":"function_call_output","call_id":call_id,"output":content}))
            }
        }
    }
    out
}

#[async_trait]
impl LlmProvider for OpenAiResponsesProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut body = json!({
            "model": req.model,
            "instructions": req.system,
            "input": to_input(&req.messages),
            "max_output_tokens": req.max_tokens,
            "store": false,
            "include": ["reasoning.encrypted_content"],
        });
        if !req.tools.is_empty() {
            body["tools"] = Value::from(
                req.tools
                    .iter()
                    .map(|t| json!({"type":"function","name":t.name,"description":t.description,"parameters":t.input_schema}))
                    .collect::<Vec<_>>(),
            );
        }
        if let Some(js) = &req.json_schema {
            body["text"] = json!({"format": {"type":"json_schema","name":js.name,"schema":js.schema,"strict":true}});
        }
        let resp = self
            .http
            .post(format!("{}/responses", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(http_err)?;
        let v = read_json(resp).await?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            return Err(LlmError::InvalidResponse(err.to_string()));
        }
        let items = v["output"].as_array().cloned().unwrap_or_default();
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for it in &items {
            match it["type"].as_str() {
                Some("message") => {
                    for c in it["content"].as_array().cloned().unwrap_or_default() {
                        if c["type"] == "output_text" {
                            content.push_str(c["text"].as_str().unwrap_or_default());
                        }
                    }
                }
                Some("function_call") => {
                    let raw = it["arguments"].as_str().unwrap_or("{}");
                    tool_calls.push(ToolCall {
                        id: it["call_id"].as_str().unwrap_or_default().to_string(),
                        name: it["name"].as_str().unwrap_or_default().to_string(),
                        arguments: serde_json::from_str(raw).unwrap_or(Value::String(raw.to_string())),
                    });
                }
                _ => {}
            }
        }
        Ok(CompletionResponse {
            content: if content.is_empty() { None } else { Some(content) },
            tool_calls,
            usage: Usage {
                input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or(0),
                output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
            },
            stop_reason: v["status"].as_str().map(String::from),
            provider_state: Some(Value::Array(items)),
        })
    }
}
