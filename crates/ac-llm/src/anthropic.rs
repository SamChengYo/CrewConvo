//! Native Anthropic Messages API.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    ChatMessage, CompletionRequest, CompletionResponse, LlmError, LlmProvider, ToolCall, Usage, http_err, read_json,
};

const STRUCTURED_TOOL: &str = "emit_result";

pub struct AnthropicProvider {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(http: reqwest::Client, api_key: String, base_url: String) -> Self {
        Self { http, api_key, base_url: base_url.trim_end_matches('/').to_string() }
    }
}

fn to_messages(msgs: &[ChatMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in msgs {
        match m {
            ChatMessage::User { content } => {
                out.push(json!({"role":"user","content":[{"type":"text","text":content}]}))
            }
            ChatMessage::Assistant { content, tool_calls, provider_state } => {
                // Replay the exact content blocks (keeps thinking blocks + signatures intact).
                if let Some(Value::Array(blocks)) = provider_state {
                    out.push(json!({"role":"assistant","content":blocks}));
                    continue;
                }
                let mut blocks = Vec::new();
                if let Some(t) = content.as_ref().filter(|t| !t.is_empty()) {
                    blocks.push(json!({"type":"text","text":t}));
                }
                for c in tool_calls {
                    blocks.push(json!({"type":"tool_use","id":c.id,"name":c.name,"input":c.arguments}));
                }
                out.push(json!({"role":"assistant","content":blocks}));
            }
            ChatMessage::Tool { call_id, content, is_error, .. } => {
                let block = json!({"type":"tool_result","tool_use_id":call_id,"content":content,"is_error":is_error});
                // Consecutive tool results go into one user turn.
                if let Some(last) = out.last_mut() {
                    let is_tool_turn = last["role"] == "user"
                        && last["content"]
                            .as_array()
                            .map(|a| a.iter().all(|b| b["type"] == "tool_result"))
                            .unwrap_or(false);
                    if is_tool_turn {
                        last["content"].as_array_mut().unwrap().push(block);
                        continue;
                    }
                }
                out.push(json!({"role":"user","content":[block]}));
            }
        }
    }
    out
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| json!({"name":t.name,"description":t.description,"input_schema":t.input_schema}))
            .collect();
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": to_messages(&req.messages),
        });
        // Structured output: force a single tool whose input is the result object.
        if let Some(js) = &req.json_schema {
            tools = vec![
                json!({"name": STRUCTURED_TOOL, "description": format!("Return the {} result.", js.name), "input_schema": js.schema}),
            ];
            body["tool_choice"] = json!({"type":"tool","name":STRUCTURED_TOOL});
        }
        if !tools.is_empty() {
            body["tools"] = Value::from(tools);
        }
        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(http_err)?;
        let v = read_json(resp).await?;
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        let blocks = v["content"].as_array().cloned().unwrap_or_default();
        for b in &blocks {
            match b["type"].as_str() {
                Some("text") => content.push_str(b["text"].as_str().unwrap_or_default()),
                Some("tool_use") => {
                    if req.json_schema.is_some() && b["name"] == STRUCTURED_TOOL {
                        content = b["input"].to_string();
                    } else {
                        tool_calls.push(ToolCall {
                            id: b["id"].as_str().unwrap_or_default().to_string(),
                            name: b["name"].as_str().unwrap_or_default().to_string(),
                            arguments: b["input"].clone(),
                        });
                    }
                }
                _ => {}
            }
        }
        if req.json_schema.is_some() {
            tool_calls.clear();
        }
        Ok(CompletionResponse {
            content: if content.is_empty() { None } else { Some(content) },
            tool_calls,
            usage: Usage {
                input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or(0)
                    + v["usage"]["cache_read_input_tokens"].as_i64().unwrap_or(0)
                    + v["usage"]["cache_creation_input_tokens"].as_i64().unwrap_or(0),
                output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
            },
            stop_reason: v["stop_reason"].as_str().map(String::from),
            provider_state: Some(Value::Array(blocks)),
        })
    }
}
