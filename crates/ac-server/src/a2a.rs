//! Wakes for platform agents, delivered with the A2A protocol (JSON-RPC binding).
//!
//! A wake becomes one `SendMessage` call (A2A 1.0; `message/send` for 0.x agents) to the agent's
//! endpoint, authenticated with the connection's access token. The answer — a Message, or a Task
//! that is polled with `GetTask` until it finishes — is posted to the conversation as the agent.
//! A stop request sends `CancelTask`. The A2A `contextId` is kept per conversation / thread so the
//! agent remembers earlier turns.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use ac_core::model::{Agent, AgentStatus, ConversationKind, MemberKind};
use ac_core::protocol::{AgentEvent, AgentReply, PayloadMessage, WakePayload};
use ac_decider::net;
use ac_service::{AgentLifecycle, Service};
use ac_wake::AgentReplier;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use uuid::Uuid;

const MAX_REPLY_CHARS: usize = 12_000;

pub struct A2aManager {
    service: Service,
    tasks: Mutex<HashMap<Uuid, JoinHandle<()>>>,
}

/// The A2A task of an in-flight wake (known once the agent answered with a non-final Task).
type TaskSlot = Arc<Mutex<Option<String>>>;

impl A2aManager {
    pub fn new(service: Service) -> Arc<Self> {
        let m = Arc::new(A2aManager { service: service.clone(), tasks: Mutex::new(HashMap::new()) });
        service.add_lifecycle(Arc::new(Hook(Arc::downgrade(&m))));
        m
    }

    pub async fn load_all(self: &Arc<Self>) -> ac_core::AppResult<()> {
        for id in self.service.store().list_a2a_agent_ids().await? {
            if let Some(a) = self.service.store().get_agent(id).await? {
                self.sync(&a);
            }
        }
        Ok(())
    }

    /// Starts or stops the connector of an agent. A running connector is left alone, so a catalog
    /// re-sync never drops queued wakes.
    pub fn sync(self: &Arc<Self>, agent: &Agent) {
        let wants = agent.delivery.as_deref() == Some("a2a")
            && agent.member.deactivated_at.is_none()
            && agent.status != AgentStatus::Disabled;
        let mut tasks = self.tasks.lock().unwrap();
        let running = tasks.get(&agent.member.id).is_some_and(|t| !t.is_finished());
        if wants && running {
            return;
        }
        if let Some(t) = tasks.remove(&agent.member.id) {
            t.abort();
        }
        if wants {
            let me = self.clone();
            let id = agent.member.id;
            tasks.insert(id, tokio::spawn(async move { me.run(id).await }));
        }
    }

    async fn run(self: Arc<Self>, agent_id: Uuid) {
        let mut link = self.service.engine().connect(agent_id);
        let mut inflight: HashMap<Uuid, (JoinHandle<()>, TaskSlot)> = HashMap::new();
        while let Some(ev) = link.events.recv().await {
            inflight.retain(|_, (h, _)| !h.is_finished());
            match ev {
                AgentEvent::Wake { payload } => {
                    let slot: TaskSlot = Arc::default();
                    let (svc, replier, s) = (self.service.clone(), link.replier.clone(), slot.clone());
                    let wake_id = payload.wake_id;
                    let h = tokio::spawn(async move { handle_wake(svc, agent_id, *payload, replier, s).await });
                    inflight.insert(wake_id, (h, slot));
                }
                AgentEvent::WakeCancelled { wake_id } => {
                    if let Some((h, slot)) = inflight.remove(&wake_id) {
                        h.abort();
                        let task = slot.lock().unwrap().clone();
                        if let Some(task_id) = task {
                            let svc = self.service.clone();
                            tokio::spawn(async move { cancel_task(svc, agent_id, task_id).await });
                        }
                    }
                }
                AgentEvent::Ping => {}
            }
        }
    }
}

struct Hook(Weak<A2aManager>);

impl AgentLifecycle for Hook {
    fn agent_changed(&self, agent: &Agent) {
        if let Some(m) = self.0.upgrade() {
            m.sync(agent);
        }
    }
}

// ---------------------------------------------------------------- wire format

/// The two wire formats: A2A 1.0 and the 0.x one still used by older agents.
#[derive(Clone, Copy)]
struct Wire {
    legacy: bool,
}

impl Wire {
    fn send(&self) -> &'static str {
        if self.legacy { "message/send" } else { "SendMessage" }
    }
    fn get(&self) -> &'static str {
        if self.legacy { "tasks/get" } else { "GetTask" }
    }
    fn cancel(&self) -> &'static str {
        if self.legacy { "tasks/cancel" } else { "CancelTask" }
    }

    fn message(&self, text: &str, context_id: Option<&str>, metadata: Value) -> Value {
        let mut m = if self.legacy {
            json!({"kind": "message", "messageId": Uuid::new_v4().to_string(), "role": "user",
                   "parts": [{"kind": "text", "text": text}]})
        } else {
            json!({"messageId": Uuid::new_v4().to_string(), "role": "ROLE_USER", "parts": [{"text": text}]})
        };
        if let Some(c) = context_id {
            m["contextId"] = json!(c);
        }
        m["metadata"] = metadata;
        m
    }
}

/// `TASK_STATE_INPUT_REQUIRED` / `input-required` → `input-required`.
fn norm_state(v: &Value) -> String {
    v.as_str().unwrap_or("").trim_start_matches("TASK_STATE_").to_ascii_lowercase().replace('_', "-")
}

fn parts_text(parts: &Value) -> String {
    parts
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["text"].as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn no_reply(v: &Value) -> bool {
    v["metadata"]["crewconvo"]["no_reply"].as_bool() == Some(true)
}

/// What an agent answered.
#[derive(Debug, Default, PartialEq)]
struct Answer {
    text: String,
    context_id: Option<String>,
    task_id: Option<String>,
    /// Normalized task state; `completed` for a direct Message.
    state: String,
    silent: bool,
}

impl Answer {
    fn pending(&self) -> bool {
        matches!(self.state.as_str(), "submitted" | "working" | "unspecified" | "")
    }
}

/// Reads the result of `SendMessage` / `GetTask` in either wire format.
fn read_result(result: &Value) -> Result<Answer, String> {
    let is_task = |v: &Value| v.get("status").is_some() || v["kind"] == "task";
    let (task, message) = if result.get("task").is_some_and(Value::is_object) {
        (Some(&result["task"]), None)
    } else if result.get("message").is_some_and(|m| m.get("parts").is_some()) {
        (None, Some(&result["message"]))
    } else if is_task(result) {
        (Some(result), None)
    } else if result.get("parts").is_some() {
        (None, Some(result))
    } else {
        return Err("回應裡沒有 task 或 message".into());
    };
    if let Some(m) = message {
        return Ok(Answer {
            text: parts_text(&m["parts"]),
            context_id: m["contextId"].as_str().map(str::to_string),
            task_id: None,
            state: "completed".into(),
            silent: no_reply(m),
        });
    }
    let t = task.expect("task");
    let mut texts: Vec<String> = Vec::new();
    let mut silent = no_reply(t);
    for a in t["artifacts"].as_array().into_iter().flatten() {
        silent |= no_reply(a);
        let s = parts_text(&a["parts"]);
        if !s.is_empty() {
            texts.push(s);
        }
    }
    if texts.is_empty() {
        let s = parts_text(&t["status"]["message"]["parts"]);
        if !s.is_empty() {
            texts.push(s);
        }
    }
    Ok(Answer {
        text: texts.join("\n\n"),
        context_id: t["contextId"].as_str().map(str::to_string),
        task_id: t["id"].as_str().map(str::to_string),
        state: norm_state(&t["status"]["state"]),
        silent,
    })
}

async fn rpc(
    service: &Service,
    url: &str,
    token: Option<&str>,
    wire: Wire,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let (u, addrs) = net::check_url(url, service.egress()).await?;
    let client = net::pinned_client(&u, &addrs, timeout)?;
    let body = json!({"jsonrpc": "2.0", "id": Uuid::new_v4().to_string(), "method": method, "params": params});
    let mut req = client.post(u).header("content-type", "application/json").header("accept", "application/json");
    if !wire.legacy {
        req = req.header("A2A-Version", "1.0");
    }
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let res = req.body(body.to_string()).send().await.map_err(|e| format!("連線失敗：{e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|_| format!("HTTP {status}，回應不是 JSON：{}", text.chars().take(200).collect::<String>()))?;
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        return Err(format!("A2A 錯誤 {}：{}", e["code"], e["message"].as_str().unwrap_or("")));
    }
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    v.get("result").cloned().ok_or_else(|| "回應缺少 result".to_string())
}

// ---------------------------------------------------------------- one wake

fn line(m: &PayloadMessage) -> String {
    let who = if m.author_kind == MemberKind::Agent { "Agent" } else { "人類" };
    format!("@{}（{who}）：{}", m.author_handle, m.body)
}

/// The text the agent receives: where it was called, what it has not seen, and what to answer.
fn build_text(p: &WakePayload, me: &str) -> String {
    let trigger = p.unread.iter().chain(p.recent.iter()).find(|m| m.id == p.trigger_message_id);
    let asker = trigger.map(|m| m.author_handle.as_str()).unwrap_or("某位成員");
    let place = match p.conversation.kind {
        ConversationKind::Channel => format!("頻道 #{}", p.conversation.name.as_deref().unwrap_or("")),
        ConversationKind::Dm => "私訊".to_string(),
    };
    let thread = if p.thread_root_id.is_some() { "的討論串" } else { "" };
    let mut out = format!("你是 CrewConvo 群聊裡的 @{me}，在{place}{thread}中被 @{asker} 呼叫。\n");
    if let Some(t) = p.conversation.topic.as_deref().filter(|t| !t.trim().is_empty()) {
        out.push_str(&format!("頻道主題：{t}\n"));
    }
    if let Some(s) = p.summary.as_deref().filter(|s| !s.trim().is_empty()) {
        out.push_str(&format!("\n先前對話的摘要：\n{s}\n"));
    }
    if let Some(r) = &p.thread_root {
        out.push_str(&format!("\n討論串的第一則訊息：\n{}\n", line(r)));
    }
    let earlier: Vec<&PayloadMessage> = p.unread.iter().filter(|m| m.id != p.trigger_message_id).collect();
    if !earlier.is_empty() {
        out.push_str("\n你還沒看過的訊息（由舊到新）：\n");
        let skip = earlier.len().saturating_sub(30);
        for m in earlier.into_iter().skip(skip) {
            out.push_str(&line(m));
            out.push('\n');
        }
    }
    if let Some(d) = &p.discussion {
        out.push_str(&format!("\n這是一場多方討論（第 {} / {} 輪）：{}\n", d.round, d.max_rounds, d.instruction));
    }
    out.push_str("\n請回覆下面這則訊息。直接寫回覆內容即可，它會以你的名義貼回同一個地方：\n");
    match trigger {
        Some(m) => out.push_str(&line(m)),
        None => out.push_str("（找不到觸發訊息，請依上面的內容回覆）"),
    }
    out
}

async fn handle_wake(service: Service, agent_id: Uuid, p: WakePayload, replier: AgentReplier, slot: TaskSlot) {
    let wake_id = p.wake_id;
    let _ = replier.reply(AgentReply::WakeAck { wake_id }).await;
    let beat = {
        let r = replier.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let _ = r.reply(AgentReply::WakeHeartbeat { wake_id }).await;
            }
        })
    };
    let log = match deliver(&service, agent_id, &p, &slot).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(agent = %agent_id, wake = %wake_id, error = %e, "A2A delivery failed");
            format!("A2A 呼叫失敗：{e}")
        }
    };
    beat.abort();
    let _ = replier.reply(AgentReply::WakeDone { wake_id, private_log: Some(log), tokens_used: None }).await;
}

async fn deliver(service: &Service, agent_id: Uuid, p: &WakePayload, slot: &TaskSlot) -> Result<String, String> {
    let store = service.store();
    let (target, token) =
        service.a2a_target(agent_id).await.map_err(|e| e.to_string())?.ok_or("這個 Agent 沒有 A2A 設定")?;
    let me = store.get_member(agent_id).await.map_err(|e| e.to_string())?.ok_or("Agent 不存在")?;
    let wire = Wire { legacy: target.legacy };
    let thread_key = p.thread_root_id.map(|t| t.to_string()).unwrap_or_else(|| "main".into());
    let context = store.a2a_context(agent_id, p.conversation.id, &thread_key).await.map_err(|e| e.to_string())?;
    let trigger = p.unread.iter().chain(p.recent.iter()).find(|m| m.id == p.trigger_message_id);
    // Who is asking and where: the platform decides permissions and billing from this.
    let meta = json!({
        "wake_id": p.wake_id,
        "reason": p.reason,
        "agent": {"id": target.external_id, "handle": me.handle},
        "workspace_id": target.external_workspace_id,
        "asker": trigger.map(|m| json!({"handle": m.author_handle, "kind": m.author_kind})),
        "conversation": {"id": p.conversation.id, "kind": p.conversation.kind, "name": p.conversation.name},
        "thread_root_id": p.thread_root_id,
    });
    let mut full = meta.clone();
    full["wake"] = serde_json::to_value(p).unwrap_or(Value::Null);
    let params = json!({
        "message": wire.message(&build_text(p, &me.handle), context.as_deref(), json!({"crewconvo": meta})),
        "metadata": {"crewconvo": full},
    });
    let budget = Duration::from_secs(p.limits.deadline_seconds.clamp(10, 3600) as u64);
    let started = Instant::now();
    let auth = token.as_deref();
    let result = rpc(service, &target.url, auth, wire, wire.send(), params, budget).await?;
    let mut ans = read_result(&result)?;
    *slot.lock().unwrap() = ans.task_id.clone();
    while ans.pending() {
        let Some(task_id) = ans.task_id.clone() else {
            return Err("Agent 回了未完成的 task 但沒有 id".into());
        };
        if started.elapsed() >= budget {
            let _ =
                rpc(service, &target.url, auth, wire, wire.cancel(), json!({"id": task_id}), Duration::from_secs(10))
                    .await;
            return Err(format!("等待 task {task_id} 逾時"));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        let r =
            rpc(service, &target.url, auth, wire, wire.get(), json!({"id": task_id}), Duration::from_secs(20)).await?;
        let next = read_result(&r)?;
        ans = Answer { context_id: next.context_id.clone().or(ans.context_id), task_id: Some(task_id), ..next };
    }
    *slot.lock().unwrap() = None;
    if let Some(c) = &ans.context_id {
        let _ = store.set_a2a_context(agent_id, p.conversation.id, &thread_key, c).await;
    }
    let task = ans.task_id.as_deref().map(|t| format!("task {t} ")).unwrap_or_default();
    if matches!(ans.state.as_str(), "failed" | "rejected" | "canceled") {
        return Ok(format!("A2A {task}結束於 {}：{}", ans.state, ans.text.chars().take(300).collect::<String>()));
    }
    if ans.silent || ans.text.trim().is_empty() {
        return Ok(format!("A2A {task}{}，Agent 選擇不回覆", ans.state));
    }
    let text: String = if ans.text.chars().count() > MAX_REPLY_CHARS {
        format!("{}…（內容過長，已截斷）", ans.text.chars().take(MAX_REPLY_CHARS).collect::<String>())
    } else {
        ans.text.clone()
    };
    service
        .post_message(&me, Some(p.wake_id), p.conversation.id, &text, p.thread_root_id)
        .await
        .map_err(|e| format!("貼回覆失敗：{e}"))?;
    Ok(format!("A2A {task}{}，已回覆 {} 字", ans.state, text.chars().count()))
}

async fn cancel_task(service: Service, agent_id: Uuid, task_id: String) {
    let Ok(Some((target, token))) = service.a2a_target(agent_id).await else { return };
    let wire = Wire { legacy: target.legacy };
    let r = rpc(
        &service,
        &target.url,
        token.as_deref(),
        wire,
        wire.cancel(),
        json!({"id": task_id}),
        Duration::from_secs(10),
    )
    .await;
    if let Err(e) = r {
        tracing::info!(agent = %agent_id, task = %task_id, error = %e, "A2A cancel failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_wire_formats() {
        let v1 = json!({"task": {"id": "t1", "contextId": "c1", "status": {"state": "TASK_STATE_COMPLETED"},
            "artifacts": [{"artifactId": "a", "parts": [{"text": "你好"}, {"text": " 世界 "}]}]}});
        let a = read_result(&v1).unwrap();
        assert_eq!((a.text.as_str(), a.state.as_str(), a.pending()), ("你好\n\n世界", "completed", false));
        assert_eq!((a.task_id.as_deref(), a.context_id.as_deref()), (Some("t1"), Some("c1")));

        let msg =
            json!({"message": {"messageId": "m", "role": "ROLE_AGENT", "parts": [{"text": "hi"}], "contextId": "c2"}});
        assert_eq!(read_result(&msg).unwrap().text, "hi");

        let old = json!({"kind": "task", "id": "t2", "status": {"state": "input-required",
            "message": {"kind": "message", "role": "agent", "parts": [{"kind": "text", "text": "哪一筆訂單？"}]}}});
        let a = read_result(&old).unwrap();
        assert_eq!((a.text.as_str(), a.state.as_str(), a.pending()), ("哪一筆訂單？", "input-required", false));

        let working = json!({"id": "t3", "status": {"state": "TASK_STATE_WORKING"}});
        assert!(read_result(&working).unwrap().pending());
        let quiet = json!({"message": {"parts": [{"text": ""}], "metadata": {"crewconvo": {"no_reply": true}}}});
        assert!(read_result(&quiet).unwrap().silent);
        assert!(read_result(&json!({"foo": 1})).is_err());
    }
}
