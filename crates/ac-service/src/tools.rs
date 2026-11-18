//! Chat tools (spec 5.2). One definition + one dispatcher, shared by the builtin runtime,
//! the agent REST API and the MCP endpoint.

use ac_core::model::{Agent, AgentOrigin, Member, MemberKind, MessageView, Visibility};
use ac_core::tool::ToolSpec;
use ac_core::{AppError, AppResult};
use ac_sandbox::SandboxLimits;
use ac_sandbox::tools::{SANDBOX_TOOLS, call_sandbox_tool, sandbox_tool_specs};
use ac_search::SearchParams;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::Service;

pub const CHAT_TOOLS: &[&str] = &[
    "list_agents",
    "list_conversations",
    "list_channels",
    "join_channel",
    "create_channel",
    "open_dm",
    "invite",
    "leave",
    "post_message",
    "react",
    "read_history",
    "search_messages",
];

fn spec(name: &str, description: &str, schema: Value) -> ToolSpec {
    ToolSpec { name: name.into(), description: description.into(), input_schema: schema }
}

const MEMBER_REF: &str = "成員 ID（UUID）或 handle（可帶 @）";

pub fn chat_tool_specs() -> Vec<ToolSpec> {
    let mut v = vec![
        spec(
            "list_agents",
            "列出你可以找的其他 Agent（不含你自己；handle、名稱、描述、專長、狀態）：同一個擁有者的 Agent 與同群組的 Agent；帶 conversation_id 時列出該對話裡的所有 Agent。可用關鍵字過濾。",
            json!({"type":"object","properties":{
                "query":{"type":"string","description":"關鍵字，比對 handle、名稱、描述、專長"},
                "conversation_id":{"type":"string","description":"只列出此對話中的 Agent"}
            }}),
        ),
        spec(
            "list_conversations",
            "列出你所在的對話（頻道與私訊），含主題與成員。",
            json!({"type":"object","properties":{}}),
        ),
        spec(
            "list_channels",
            "列出你看得到的頻道：你已加入的頻道，加上所有 Agent 共用的大廳 #general；附主題、成員數、裡面有哪些 Agent、你是否已加入。其他頻道對 Agent 不可見，需要被邀請才會出現。要找討論的地方時先用這個。",
            json!({"type":"object","properties":{"query":{"type":"string","description":"關鍵字，比對頻道名稱與主題"}}}),
        ),
        spec(
            "join_channel",
            "加入大廳 #general（Agent 只能自行加入這個頻道，其他頻道需要被邀請）。可用頻道 ID 或名稱（可帶 #）。",
            json!({"type":"object","properties":{"channel":{"type":"string"}},"required":["channel"]}),
        ),
        spec(
            "create_channel",
            "建立頻道，你會自動加入。",
            json!({"type":"object","properties":{
                "name":{"type":"string"},
                "topic":{"type":"string"},
                "visibility":{"type":"string","enum":["public","private"]},
                "invite":{"type":"array","items":{"type":"string"},"description":MEMBER_REF}
            },"required":["name","visibility"]}),
        ),
        spec(
            "open_dm",
            "取得或建立與指定成員的私訊。",
            json!({"type":"object","properties":{
                "member_ids":{"type":"array","items":{"type":"string"},"description":MEMBER_REF}
            },"required":["member_ids"]}),
        ),
        spec(
            "invite",
            "邀請成員加入你所在的頻道。",
            json!({"type":"object","properties":{
                "conversation_id":{"type":"string"},
                "member_ids":{"type":"array","items":{"type":"string"},"description":MEMBER_REF}
            },"required":["conversation_id","member_ids"]}),
        ),
        spec(
            "leave",
            "離開對話。",
            json!({"type":"object","properties":{"conversation_id":{"type":"string"}},"required":["conversation_id"]}),
        ),
        spec(
            "post_message",
            "在對話中發訊息。這是唯一的發言方式；不呼叫就不會發言。body 中的 @handle 會通知該成員。",
            json!({"type":"object","properties":{
                "conversation_id":{"type":"string"},
                "body":{"type":"string"},
                "thread_root_id":{"type":"string","description":"在此訊息的 thread 中回覆"}
            },"required":["conversation_id","body"]}),
        ),
        spec(
            "react",
            "對訊息加上表情反應（例如表示已收到）。不會喚醒任何人。",
            json!({"type":"object","properties":{
                "message_id":{"type":"string"},
                "emoji":{"type":"string"}
            },"required":["message_id","emoji"]}),
        ),
        spec(
            "read_history",
            "分頁讀取對話歷史（由新到舊取 limit 則，回傳時按時間排序）。",
            json!({"type":"object","properties":{
                "conversation_id":{"type":"string"},
                "thread_root_id":{"type":"string"},
                "before_seq":{"type":"integer"},
                "limit":{"type":"integer","minimum":1,"maximum":100}
            },"required":["conversation_id"]}),
        ),
    ];
    v.push(search_tool_spec());
    v
}

/// Tools allowed by an agent's `tools` config: an empty list means all chat tools.
pub fn search_tool_spec() -> ToolSpec {
    spec(
        "search_messages",
        "全文搜尋你所在對話的訊息（支援中文、\"片語\"）。回傳命中訊息、所在對話、前後各 1 則語境與相關度。",
        json!({"type":"object","properties":{
            "query":{"type":"string","description":"關鍵字；用雙引號包住表示片語"},
            "conversation_id":{"type":"string"},
            "author":{"type":"string","description":"作者 handle"},
            "after":{"type":"string","description":"RFC 3339 時間"},
            "before":{"type":"string","description":"RFC 3339 時間"},
            "limit":{"type":"integer","minimum":1,"maximum":50},
            "offset":{"type":"integer","minimum":0},
            "mode":{"type":"string","enum":["keyword","semantic"]}
        },"required":["query"]}),
    )
}

pub fn allowed_chat_tools(config: &Value) -> Vec<ToolSpec> {
    let allowed: Vec<String> =
        config.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    chat_tool_specs().into_iter().filter(|t| allowed.is_empty() || allowed.iter().any(|a| a == &t.name)).collect()
}

/// Every tool an agent may call: chat tools (all agents) + sandbox tools (builtin agents,
/// when a sandbox backend is configured). An agent's `tools` list restricts both; an empty
/// list means everything available.
pub fn agent_tool_specs(agent: &Agent, sandbox_available: bool) -> Vec<ToolSpec> {
    let allowed: Vec<String> = agent
        .tools
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let mut all = chat_tool_specs();
    if agent.origin == AgentOrigin::Builtin && sandbox_available {
        all.extend(sandbox_tool_specs());
    }
    all.into_iter().filter(|t| allowed.is_empty() || allowed.iter().any(|a| a == &t.name)).collect()
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> AppResult<T> {
    let args = if args.is_null() { json!({}) } else { args };
    serde_json::from_value(args).map_err(|e| AppError::bad_request(format!("參數錯誤：{e}")))
}

fn parse_uuid(s: &str, what: &str) -> AppResult<Uuid> {
    s.trim().parse().map_err(|_| AppError::bad_request(format!("{what} 不是有效的 ID")))
}

fn compact(m: &MessageView) -> Value {
    json!({
        "id": m.id, "seq": m.seq, "author_handle": m.author_handle, "author_kind": m.author_kind,
        "body": m.body, "thread_root_id": m.thread_root_id, "created_at": m.created_at,
        "mentions": m.mentions, "reactions": m.reactions, "reply_count": m.reply_count,
    })
}

impl Service {
    async fn resolve_member_refs(&self, actor: &Member, refs: &[String]) -> AppResult<Vec<Uuid>> {
        let mut ids = Vec::new();
        let mut handles = Vec::new();
        for r in refs {
            let r = r.trim();
            match r.parse::<Uuid>() {
                Ok(id) => ids.push(id),
                Err(_) => handles.push(r.trim_start_matches('@').to_lowercase()),
            }
        }
        if !handles.is_empty() {
            let found = self.inner.store.members_by_handles(actor.workspace_id, &handles).await?;
            for h in &handles {
                match found.iter().find(|m| m.handle.to_lowercase() == *h && m.kind != MemberKind::System) {
                    Some(m) => ids.push(m.id),
                    None => return Err(AppError::bad_request(format!("找不到成員 @{h}"))),
                }
            }
        }
        // An agent can only reach agents it can see (same owner or a shared group).
        if let Some(visible) = self.visible_agents_for(actor).await? {
            for id in &ids {
                if let Some(m) = self.inner.store.get_member(*id).await?
                    && m.kind == MemberKind::Agent
                    && !visible.contains(id)
                {
                    return Err(AppError::bad_request(format!(
                        "找不到成員 @{}（你只能找同一個擁有者或同一個群組的 Agent）",
                        m.handle
                    )));
                }
            }
        }
        Ok(ids)
    }

    /// Executes a chat tool on behalf of `actor`. `wake_id` is bound by the transport
    /// (runtime / connection / MCP session), never taken from tool arguments.
    pub async fn call_tool(&self, actor: &Member, wake_id: Option<Uuid>, name: &str, args: Value) -> AppResult<Value> {
        if actor.kind == MemberKind::Agent
            && let Some(agent) = self.inner.store.get_agent(actor.id).await?
        {
            let allowed: Vec<&str> =
                agent.tools.as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
            if !allowed.is_empty() && !allowed.contains(&name) {
                return Err(AppError::forbidden(format!("此 Agent 沒有啟用工具 {name}")));
            }
        }
        match name {
            "list_agents" => {
                #[derive(Deserialize)]
                struct A {
                    query: Option<String>,
                    conversation_id: Option<String>,
                }
                let a: A = parse(args)?;
                let conv = a.conversation_id.as_deref().map(|c| parse_uuid(c, "conversation_id")).transpose()?;
                let agents = self.list_agents(actor, a.query.as_deref(), conv).await?;
                Ok(json!({ "agents": agents }))
            }
            "list_conversations" => {
                let convs = self.list_conversations(actor).await?;
                let mut out = Vec::new();
                for c in convs {
                    let members = self.inner.store.conversation_members(c.conversation.id).await?;
                    out.push(json!({
                        "id": c.conversation.id, "kind": c.conversation.kind, "name": c.conversation.name,
                        "topic": c.conversation.topic, "visibility": c.conversation.visibility,
                        "members": members.iter().map(|m| m.handle.clone()).collect::<Vec<_>>(),
                    }));
                }
                Ok(json!({ "conversations": out }))
            }
            "create_channel" => {
                #[derive(Deserialize)]
                struct A {
                    name: String,
                    topic: Option<String>,
                    visibility: Visibility,
                    #[serde(default)]
                    invite: Vec<String>,
                }
                let a: A = parse(args)?;
                let invite = self.resolve_member_refs(actor, &a.invite).await?;
                let c = self.create_channel(actor, &a.name, a.topic.as_deref(), a.visibility, &invite).await?;
                Ok(json!({ "conversation_id": c.id, "name": c.name }))
            }
            "open_dm" => {
                #[derive(Deserialize)]
                struct A {
                    member_ids: Vec<String>,
                }
                let a: A = parse(args)?;
                let ids = self.resolve_member_refs(actor, &a.member_ids).await?;
                let c = self.open_dm(actor, &ids).await?;
                Ok(json!({ "conversation_id": c.id }))
            }
            "invite" => {
                #[derive(Deserialize)]
                struct A {
                    conversation_id: String,
                    member_ids: Vec<String>,
                }
                let a: A = parse(args)?;
                let ids = self.resolve_member_refs(actor, &a.member_ids).await?;
                let added = self.invite(actor, parse_uuid(&a.conversation_id, "conversation_id")?, &ids).await?;
                Ok(json!({ "added": added }))
            }
            "list_channels" => {
                #[derive(Deserialize)]
                struct A {
                    query: Option<String>,
                }
                let a: A = parse(args)?;
                Ok(json!({ "channels": self.channel_directory(actor, a.query.as_deref()).await? }))
            }
            "join_channel" => {
                #[derive(Deserialize)]
                struct A {
                    channel: String,
                }
                let a: A = parse(args)?;
                let c = self.join_channel_ref(actor, &a.channel).await?;
                Ok(json!({ "conversation_id": c.id, "name": c.name, "topic": c.topic }))
            }
            "leave" => {
                #[derive(Deserialize)]
                struct A {
                    conversation_id: String,
                }
                let a: A = parse(args)?;
                self.leave(actor, parse_uuid(&a.conversation_id, "conversation_id")?).await?;
                Ok(json!({ "ok": true }))
            }
            "post_message" => {
                #[derive(Deserialize)]
                struct A {
                    conversation_id: String,
                    body: String,
                    thread_root_id: Option<String>,
                }
                let a: A = parse(args)?;
                let thread = a
                    .thread_root_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|t| parse_uuid(t, "thread_root_id"))
                    .transpose()?;
                let r = self
                    .post_message(actor, wake_id, parse_uuid(&a.conversation_id, "conversation_id")?, &a.body, thread)
                    .await?;
                Ok(json!({
                    "message_id": r.message.id, "seq": r.message.seq,
                    "ignored_mentions": r.ignored_mentions,
                }))
            }
            "react" => {
                #[derive(Deserialize)]
                struct A {
                    message_id: String,
                    emoji: String,
                }
                let a: A = parse(args)?;
                self.react(actor, parse_uuid(&a.message_id, "message_id")?, &a.emoji, true).await?;
                Ok(json!({ "ok": true }))
            }
            "read_history" => {
                #[derive(Deserialize)]
                struct A {
                    conversation_id: String,
                    thread_root_id: Option<String>,
                    before_seq: Option<i64>,
                    limit: Option<i64>,
                }
                let a: A = parse(args)?;
                let thread = a
                    .thread_root_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|t| parse_uuid(t, "thread_root_id"))
                    .transpose()?;
                let msgs = self
                    .read_history(
                        actor,
                        parse_uuid(&a.conversation_id, "conversation_id")?,
                        thread,
                        a.before_seq,
                        a.limit,
                    )
                    .await?;
                Ok(json!({ "messages": msgs.iter().map(compact).collect::<Vec<_>>() }))
            }
            "search_messages" => {
                let p: SearchParams = parse(args)?;
                let r = self.inner.search.search(actor, &p).await?;
                Ok(json!({
                    "total": r.total,
                    "offset": r.offset,
                    "hits": r.hits.iter().map(|h| json!({
                        "message": compact(&h.message),
                        "conversation": h.conversation,
                        "before": h.before.as_ref().map(compact),
                        "after": h.after.as_ref().map(compact),
                        "score": h.score,
                    })).collect::<Vec<_>>(),
                }))
            }
            name if SANDBOX_TOOLS.contains(&name) => {
                let agent = self
                    .inner
                    .store
                    .get_agent(actor.id)
                    .await?
                    .ok_or_else(|| AppError::forbidden("只有 Agent 有沙箱"))?;
                if agent.origin != AgentOrigin::Builtin {
                    return Err(AppError::forbidden("沙箱工具只提供給內建 Agent"));
                }
                let sb = self.inner.sandbox.clone().ok_or_else(|| AppError::bad_request("此伺服器沒有啟用沙箱"))?;
                let limits = SandboxLimits::from_json(&agent.sandbox_config);
                call_sandbox_tool(sb.as_ref(), agent.member.id, &limits, name, args)
                    .await
                    .map_err(|e| AppError::BadRequest(e.to_string()))
            }
            other => Err(AppError::bad_request(format!("未知的工具：{other}"))),
        }
    }
}
