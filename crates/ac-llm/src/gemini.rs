//! Native Gemini `generateContent`. Model turns are replayed verbatim so that
//! `thoughtSignature`s on function calls survive across turns.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    ChatMessage, CompletionRequest, CompletionResponse, LlmError, LlmProvider, ToolCall, Usage, http_err, read_json,
};

pub struct GeminiProvider {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl GeminiProvider {
    pub fn new(http: reqwest::Client, api_key: String, base_url: String) -> Self {
        Self { http, api_key, base_url: base_url.trim_end_matches('/').to_string() }
    }
}

/// Gemini wants function responses as objects.
fn response_object(content: &str) -> Value {
    match serde_json::from_str::<Value>(content) {
        Ok(v @ Value::Object(_)) => v,
        Ok(other) => json!({ "result": other }),
        Err(_) => json!({ "result": content }),
    }
}

fn to_contents(msgs: &[ChatMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in msgs {
        match m {
            ChatMessage::User { content } => out.push(json!({"role":"user","parts":[{"text":content}]})),
            ChatMessage::Assistant { content, tool_calls, provider_state } => {
                if let Some(v @ Value::Object(_)) = provider_state {
                    out.push(v.clone());
                    continue;
                }
                let mut parts = Vec::new();
                if let Some(t) = content.as_ref().filter(|t| !t.is_empty()) {
                    parts.push(json!({"text":t}));
                }
                for c in tool_calls {
                    parts.push(json!({"functionCall":{"id":c.id,"name":c.name,"args":c.arguments}}));
                }
                out.push(json!({"role":"model","parts":parts}));
            }
            ChatMessage::Tool { call_id, name, content, .. } => {
                let part = json!({"functionResponse":{"id":call_id,"name":name,"response":response_object(content)}});
                if let Some(last) = out.last_mut() {
                    let is_resp_turn = last["role"] == "user"
                        && last["parts"]
                            .as_array()
                            .map(|a| a.iter().all(|p| p.get("functionResponse").is_some()))
                            .unwrap_or(false);
                    if is_resp_turn {
                        last["parts"].as_array_mut().unwrap().push(part);
                        continue;
                    }
                }
                out.push(json!({"role":"user","parts":[part]}));
            }
        }
    }
    out
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut generation = json!({"maxOutputTokens": req.max_tokens});
        if let Some(js) = &req.json_schema {
            generation["responseMimeType"] = json!("application/json");
            generation["responseJsonSchema"] = js.schema.clone();
        }
        let mut body = json!({
            "systemInstruction": {"parts":[{"text": req.system}]},
            "contents": to_contents(&req.messages),
            "generationConfig": generation,
        });
        if !req.tools.is_empty() {
            body["tools"] = json!([{ "functionDeclarations": req.tools.iter()
                .map(|t| json!({"name":t.name,"description":t.description,"parameters":t.input_schema}))
                .collect::<Vec<_>>() }]);
        }
        let resp = self
            .http
            .post(format!("{}/models/{}:generateContent", self.base_url, req.model))
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(http_err)?;
        let v = read_json(resp).await?;
        let cand = &v["candidates"][0];
        let model_content = cand["content"].clone();
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for (i, p) in model_content["parts"].as_array().cloned().unwrap_or_default().iter().enumerate() {
            if p["thought"] == true {
                continue;
            }
            if let Some(t) = p["text"].as_str() {
                content.push_str(t);
            }
            if let Some(fc) = p.get("functionCall") {
                tool_calls.push(ToolCall {
                    id: fc["id"].as_str().map(String::from).unwrap_or_else(|| format!("call_{i}")),
                    name: fc["name"].as_str().unwrap_or_default().to_string(),
                    arguments: fc["args"].clone(),
                });
            }
        }
        if model_content.is_null() && cand["finishReason"].is_string() && cand["finishReason"] != "STOP" {
            return Err(LlmError::InvalidResponse(format!("gemini finishReason {}", cand["finishReason"])));
        }
        let usage = &v["usageMetadata"];
        Ok(CompletionResponse {
            content: if content.is_empty() { None } else { Some(content) },
            tool_calls,
            usage: Usage {
                input_tokens: usage["promptTokenCount"].as_i64().unwrap_or(0),
                output_tokens: usage["candidatesTokenCount"].as_i64().unwrap_or(0)
                    + usage["thoughtsTokenCount"].as_i64().unwrap_or(0),
            },
            stop_reason: cand["finishReason"].as_str().map(String::from),
            provider_state: if model_content.is_object() { Some(model_content) } else { None },
        })
    }
}
