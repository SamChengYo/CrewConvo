//! Minimal external agent for CrewConvo.
//!
//! 1. Connects to `GET /agent/v1/ws` with `Authorization: Bearer <token>`.
//! 2. On `wake`: replies `wake.ack`, decides whether to speak, acts through
//!    `POST /agent/v1/tools/<tool>` (actions during a wake are bound to it automatically),
//!    then sends `wake.done` with a private note.
//! 3. On `wake.cancelled`: stops.
//!
//! Behaviour of this example: if the triggering message contains a question mark it answers in
//! the same place (thread or main) with a short acknowledgement and the number of unread messages;
//! otherwise it only reacts 👀 — showing that "not speaking" is a normal outcome.
//!
//! Usage: crewconvo-example-agent --server http://localhost:8090 --token ccat_…

use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[derive(Parser)]
struct Args {
    /// Base URL of the CrewConvo server
    #[arg(long, env = "CREWCONVO_SERVER", default_value = "http://localhost:8090")]
    server: String,
    /// Agent token (ccat_…)
    #[arg(long, env = "CREWCONVO_AGENT_TOKEN")]
    token: String,
}

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    async fn tool(&self, name: &str, args: Value) -> anyhow::Result<Value> {
        let r = self
            .http
            .post(format!("{}/agent/v1/tools/{name}", self.base))
            .bearer_auth(&self.token)
            .json(&args)
            .send()
            .await?;
        let status = r.status();
        let v: Value = r.json().await?;
        anyhow::ensure!(status.is_success(), "{name}: {status} {v}");
        Ok(v["result"].clone())
    }
}

async fn handle_wake(c: &Client, payload: &Value) -> anyhow::Result<String> {
    let conv = &payload["conversation"]["id"];
    let trigger = payload["trigger_message_id"].as_str().unwrap_or_default();
    let unread = payload["unread"].as_array().cloned().unwrap_or_default();
    let trigger_msg = unread.iter().find(|m| m["id"] == trigger).cloned().unwrap_or(json!({}));
    let body = trigger_msg["body"].as_str().unwrap_or_default();
    if body.contains('?') || body.contains('？') {
        let reply = format!(
            "收到 @{} 的問題，我看了 {} 則未讀訊息。這是範例外部 Agent 的自動回覆。",
            trigger_msg["author_handle"].as_str().unwrap_or(""),
            unread.len()
        );
        c.tool(
            "post_message",
            json!({"conversation_id": conv, "body": reply, "thread_root_id": payload["thread_root_id"]}),
        )
        .await?;
        Ok("回覆了一則問題".into())
    } else {
        c.tool("react", json!({"message_id": trigger, "emoji": "👀"})).await?;
        Ok("不是問題，只按了表情".into())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let base = args.server.trim_end_matches('/').to_string();
    let client = Client { http: reqwest::Client::new(), base: base.clone(), token: args.token.clone() };
    loop {
        let ws_url = format!("{}/agent/v1/ws", base.replacen("http", "ws", 1));
        let mut req = ws_url.as_str().into_client_request()?;
        req.headers_mut().insert("authorization", format!("Bearer {}", args.token).parse()?);
        let (mut ws, _) = match tokio_tungstenite::connect_async(req).await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("connect failed: {e}; retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        println!("connected to {ws_url}");
        while let Some(msg) = ws.next().await {
            let Message::Text(text) = msg.context("websocket")? else { continue };
            let ev: Value = serde_json::from_str(text.as_str())?;
            match ev["type"].as_str() {
                Some("hello") => println!("hello: {}", ev["agent"]),
                Some("wake") => {
                    let payload = &ev["payload"];
                    let wake_id = payload["wake_id"].clone();
                    println!("wake {wake_id} ({})", payload["reason"]);
                    ws.send(Message::Text(json!({"type":"wake.ack","wake_id":wake_id}).to_string().into())).await?;
                    let note = match handle_wake(&client, payload).await {
                        Ok(n) => n,
                        Err(e) => format!("失敗：{e}"),
                    };
                    ws.send(Message::Text(
                        json!({"type":"wake.done","wake_id":wake_id,"private_log":note,"tokens_used":0})
                            .to_string()
                            .into(),
                    ))
                    .await?;
                }
                Some("wake.cancelled") => println!("cancelled {}", ev["wake_id"]),
                Some("ping") => {}
                other => println!("event {other:?}: {ev}"),
            }
        }
        eprintln!("disconnected; reconnecting in 3s");
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
