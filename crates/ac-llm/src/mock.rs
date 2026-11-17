//! Deterministic scripted "model" for tests and offline demos.
//!
//! Behaviours (model id after `mock:`):
//! - `reply`          — posts one message in the trigger conversation/thread; when the payload
//!   carries a discussion conclusion instruction, posts the conclusion in the thread and in the
//!   main conversation
//! - `react`          — reacts 👍 to the trigger message, never posts
//! - `silent`         — does nothing (final text only → private_log)
//! - `slow`           — waits a long time (cancellation / timeout tests)
//! - `mention:<h>`    — posts a message tagging `@<h>`
//! - `search:<q>`     — calls `search_messages` with `<q>`, then posts what it found
//! - `summarize`      — plain completion: a deterministic summary of the user text

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{ChatMessage, CompletionRequest, CompletionResponse, LlmError, LlmProvider, ToolCall, Usage};

pub const PAYLOAD_OPEN: &str = "<wake_payload>";
pub const PAYLOAD_CLOSE: &str = "</wake_payload>";

pub struct MockProvider;

fn payload(req: &CompletionRequest) -> Option<Value> {
    req.messages.iter().find_map(|m| match m {
        ChatMessage::User { content } => {
            let start = content.find(PAYLOAD_OPEN)? + PAYLOAD_OPEN.len();
            let end = content.find(PAYLOAD_CLOSE)?;
            serde_json::from_str(content[start..end].trim()).ok()
        }
        _ => None,
    })
}

fn usage(req: &CompletionRequest) -> Usage {
    let chars: usize = req
        .messages
        .iter()
        .map(|m| match m {
            ChatMessage::User { content } => content.len(),
            ChatMessage::Tool { content, .. } => content.len(),
            ChatMessage::Assistant { content, .. } => content.as_ref().map(|c| c.len()).unwrap_or(0),
        })
        .sum();
    Usage { input_tokens: (chars / 4) as i64 + 50, output_tokens: 20 }
}

fn calls(list: Vec<(&str, Value)>) -> CompletionResponse {
    CompletionResponse {
        content: None,
        tool_calls: list
            .into_iter()
            .enumerate()
            .map(|(i, (name, args))| ToolCall { id: format!("call_{i}_{name}"), name: name.into(), arguments: args })
            .collect(),
        usage: Usage::default(),
        stop_reason: Some("tool_use".into()),
        provider_state: None,
    }
}

fn finish(text: &str) -> CompletionResponse {
    CompletionResponse {
        content: Some(text.into()),
        tool_calls: vec![],
        usage: Usage::default(),
        stop_reason: Some("end_turn".into()),
        provider_state: None,
    }
}

fn last_tool_result(req: &CompletionRequest) -> Option<Value> {
    req.messages.iter().rev().find_map(|m| match m {
        ChatMessage::Tool { content, .. } => serde_json::from_str(content).ok(),
        _ => None,
    })
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req.model.clone();
        if model == "summarize" {
            let text = req
                .messages
                .iter()
                .filter_map(|m| match m {
                    ChatMessage::User { content } => Some(content.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
            let head: String = text.chars().filter(|c| *c != '\n').take(300).collect();
            let mut r = finish(&format!("【mock 摘要】共 {lines} 行。開頭：{head}"));
            r.usage = usage(&req);
            return Ok(r);
        }

        let p = payload(&req).ok_or_else(|| LlmError::InvalidResponse("mock: no wake payload".into()))?;
        let already_acted = req.messages.iter().any(|m| matches!(m, ChatMessage::Tool { .. }));
        let conv = p["conversation"]["id"].clone();
        let thread = p["thread_root_id"].clone();
        let depth = p["limits"]["depth"].as_i64().unwrap_or(0);
        let conclusion = p["discussion"]["phase"] == "conclusion";

        let mut resp = if model == "slow" {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            finish("mock: slow done")
        } else if let Some(q) = model.strip_prefix("search:") {
            if !already_acted {
                calls(vec![("search_messages", json!({"query": q, "limit": 5}))])
            } else if req.messages.iter().filter(|m| matches!(m, ChatMessage::Tool { .. })).count() == 1 {
                let found = last_tool_result(&req)
                    .and_then(|v| v["hits"][0]["message"]["body"].as_str().map(String::from))
                    .unwrap_or_else(|| "（沒有找到）".into());
                calls(vec![(
                    "post_message",
                    json!({"conversation_id": conv, "body": format!("找到：{found}"), "thread_root_id": thread}),
                )])
            } else {
                finish("mock: 搜尋完成")
            }
        } else if already_acted {
            finish("mock: 本回合完成")
        } else if model == "reply" && conclusion {
            calls(vec![
                (
                    "post_message",
                    json!({"conversation_id": conv, "body": "mock 結論：討論結束", "thread_root_id": thread}),
                ),
                ("post_message", json!({"conversation_id": conv, "body": "mock 結論（同步到主對話）：討論結束"})),
            ])
        } else if model == "reply" {
            calls(vec![(
                "post_message",
                json!({"conversation_id": conv, "body": format!("mock 回覆（depth {}）", depth + 1), "thread_root_id": thread}),
            )])
        } else if model == "react" {
            calls(vec![("react", json!({"message_id": p["trigger_message_id"], "emoji": "👍"}))])
        } else if model == "silent" {
            finish("mock: 判斷不需要回覆")
        } else if let Some(h) = model.strip_prefix("mention:") {
            calls(vec![(
                "post_message",
                json!({"conversation_id": conv, "body": format!("@{h} 請你看一下"), "thread_root_id": thread}),
            )])
        } else {
            return Err(LlmError::InvalidResponse(format!("unknown mock behaviour: {model}")));
        };
        resp.usage = usage(&req);
        Ok(resp)
    }
}
