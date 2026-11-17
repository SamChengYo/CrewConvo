//! Phase 4 acceptance tests (spec 13): external agents (WebSocket, REST, MCP, webhook)
//! and sandboxes.

mod common;

use std::sync::Arc;
use std::time::Duration;

use ac_core::model::{AgentStatus, MemberKind, WakeStatus};
use ac_service::external::CreateExternalAgentInput;
use ac_store::MessageScope;
use axum::Router;
use axum::routing::post;
use common::{Opts, fixture, fixture_with, wait_for};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn ext_input(handle: &str, delivery: &str, webhook_url: Option<String>) -> CreateExternalAgentInput {
    CreateExternalAgentInput {
        handle: handle.into(),
        display_name: None,
        description: "外部 Agent，負責整理會議紀錄".into(),
        expertise: vec!["會議紀錄".into()],
        delivery: Some(delivery.into()),
        webhook_url,
        max_wake_seconds: Some(60),
        daily_token_budget: None,
        join_conversations: vec![],
    }
}

async fn tool(c: &reqwest::Client, base: &str, token: &str, name: &str, args: Value) -> Value {
    let r = c.post(format!("{base}/agent/v1/tools/{name}")).bearer_auth(token).json(&args).send().await.unwrap();
    let status = r.status();
    let v: Value = r.json().await.unwrap();
    assert!(status.is_success(), "{name} failed: {status} {v}");
    v["result"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn external_agent_over_websocket_reads_creates_invites_and_talks() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let creds = fx.svc().create_external_agent(&fx.human, ext_input("scribe", "websocket", None)).await.unwrap();
    let token = creds.token.clone().unwrap();
    assert!(token.starts_with("ccat_"));
    let ext_id = creds.agent.member.id;
    assert_eq!(creds.agent.status, AgentStatus::Offline);
    let (helper, _) = fx.agent("helper", "mock:reply").await;
    let ch = fx.channel(&[ext_id]).await;
    fx.svc().post_message(&fx.human, None, ch.id, "會議開始前的背景資料", None).await.unwrap();

    // Connect.
    let mut req = format!("{}/agent/v1/ws", base.replace("http", "ws")).into_client_request().unwrap();
    req.headers_mut().insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let hello: Value = match ws.next().await.unwrap().unwrap() {
        WsMsg::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(hello["type"], "hello");
    wait_for("online", Duration::from_secs(5), || async {
        fx.store().get_agent(ext_id).await.unwrap().unwrap().status == AgentStatus::Online
    })
    .await;

    // Tagged → wake event.
    let trigger = fx.svc().post_message(&fx.human, None, ch.id, "@scribe 請整理一下", None).await.unwrap().message;
    let wake: Value = loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.unwrap().unwrap().unwrap();
        if let WsMsg::Text(t) = msg {
            let v: Value = serde_json::from_str(t.as_str()).unwrap();
            if v["type"] == "wake" {
                break v;
            }
        }
    };
    let payload = &wake["payload"];
    let wake_id = payload["wake_id"].as_str().unwrap().to_string();
    assert_eq!(payload["trigger_message_id"], trigger.id.to_string());
    assert!(payload["unread"].as_array().unwrap().iter().any(|m| m["body"] == "會議開始前的背景資料"));
    ws.send(WsMsg::Text(json!({"type":"wake.ack","wake_id":wake_id}).to_string().into())).await.unwrap();

    // Actions over REST — implicitly bound to the running wake.
    let c = reqwest::Client::new();
    let hist = tool(&c, &base, &token, "read_history", json!({"conversation_id": ch.id})).await;
    assert!(hist["messages"].as_array().unwrap().len() >= 2);
    let agents = tool(&c, &base, &token, "list_agents", json!({"query": "helper"})).await;
    assert_eq!(agents["agents"][0]["handle"], "helper");
    let created = tool(&c, &base, &token, "create_channel", json!({"name": format!("notes-{}", uuid::Uuid::now_v7().simple()), "visibility": "public", "invite": ["@helper"]})).await;
    let new_ch: uuid::Uuid = created["conversation_id"].as_str().unwrap().parse().unwrap();
    let posted =
        tool(&c, &base, &token, "post_message", json!({"conversation_id": new_ch, "body": "@helper 請幫我補充"})).await;
    let posted_id: uuid::Uuid = posted["message_id"].as_str().unwrap().parse().unwrap();
    let m = fx.store().get_message(posted_id).await.unwrap().unwrap();
    assert_eq!(m.depth, 1, "bound to the wake: depth = wake depth + 1");
    assert_eq!(m.wake_id.map(|w| w.to_string()), Some(wake_id.clone()));

    // The builtin agent answers in the new channel.
    wait_for("helper replies", Duration::from_secs(20), || async {
        fx.store()
            .list_messages(new_ch, MessageScope::Main, None, 10)
            .await
            .unwrap()
            .iter()
            .any(|m| m.author_id == helper.member.id)
    })
    .await;

    ws.send(WsMsg::Text(
        json!({"type":"wake.done","wake_id":wake_id,"private_log":"整理完成","tokens_used":123}).to_string().into(),
    ))
    .await
    .unwrap();
    wait_for("wake done", Duration::from_secs(10), || async {
        let w = fx.store().get_wake(wake_id.parse().unwrap()).await.unwrap().unwrap();
        w.status == WakeStatus::Done && w.tokens_used == 123 && w.posted_count == 1
    })
    .await;
    drop(ws);
    wait_for("offline after disconnect", Duration::from_secs(5), || async {
        fx.store().get_agent(ext_id).await.unwrap().unwrap().status == AgentStatus::Offline
    })
    .await;
}

fn sse_json(body: &str) -> Vec<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        return vec![v];
    }
    body.lines().filter_map(|l| l.strip_prefix("data:")).filter_map(|d| serde_json::from_str(d.trim()).ok()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn external_agent_over_mcp_lists_and_calls_tools() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let creds = fx.svc().create_external_agent(&fx.human, ext_input("mcpbot", "websocket", None)).await.unwrap();
    let token = creds.token.unwrap();
    let ch = fx.channel(&[creds.agent.member.id]).await;
    let c = reqwest::Client::new();
    let rpc = |body: Value, session: Option<String>| {
        let mut r = c
            .post(format!("{base}/mcp"))
            .bearer_auth(&token)
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("mcp-protocol-version", "2025-06-18")
            .json(&body);
        if let Some(s) = session {
            r = r.header("mcp-session-id", s);
        }
        r.send()
    };
    // Unauthenticated → 401.
    let anon = c.post(format!("{base}/mcp")).json(&json!({})).send().await.unwrap();
    assert_eq!(anon.status(), 401);

    let init = rpc(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        None,
    )
    .await
    .unwrap();
    assert!(init.status().is_success(), "{}", init.status());
    let session = init.headers().get("mcp-session-id").map(|v| v.to_str().unwrap().to_string());
    let body = init.text().await.unwrap();
    assert!(sse_json(&body).iter().any(|v| v["result"]["serverInfo"]["name"] == "crewconvo"), "{body}");
    rpc(json!({"jsonrpc":"2.0","method":"notifications/initialized"}), session.clone()).await.unwrap();

    let list = rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}), session.clone())
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let tools = sse_json(&list).into_iter().find(|v| v["id"] == 2).unwrap();
    let names: Vec<String> =
        tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    for n in ["post_message", "search_messages", "read_history", "create_channel", "react"] {
        assert!(names.contains(&n.to_string()), "{names:?}");
    }
    assert!(!names.contains(&"bash".to_string()), "external agents have no sandbox tools");

    let call = rpc(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"post_message","arguments":{"conversation_id": ch.id, "body":"從 MCP 發的訊息"}}}),
        session.clone(),
    )
    .await
    .unwrap()
    .text()
    .await
    .unwrap();
    let res = sse_json(&call).into_iter().find(|v| v["id"] == 3).unwrap();
    assert_ne!(res["result"]["isError"], true, "{res}");
    let msgs = fx.store().list_messages(ch.id, MessageScope::Main, None, 5).await.unwrap();
    assert!(msgs.iter().any(|m| m.body == "從 MCP 發的訊息" && m.author_kind == MemberKind::Agent));

    // Errors come back as tool errors, not protocol failures.
    let bad = rpc(
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"read_history","arguments":{"conversation_id": uuid::Uuid::now_v7()}}}),
        session,
    )
    .await
    .unwrap()
    .text()
    .await
    .unwrap();
    let res = sse_json(&bad).into_iter().find(|v| v["id"] == 4).unwrap();
    assert_eq!(res["result"]["isError"], true, "{res}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webhook_agent_receives_signed_wakes_and_replies_over_rest() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let (tx, mut rx) = mpsc::unbounded_channel::<(axum::http::HeaderMap, String)>();
    let hook = Router::new().route(
        "/hook",
        post(move |headers: axum::http::HeaderMap, body: String| {
            let tx = tx.clone();
            async move {
                let _ = tx.send((headers, body));
                "ok"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, hook).await.unwrap() });

    let creds = fx.svc().create_external_agent(&fx.human, ext_input("hooky", "webhook", Some(hook_url))).await.unwrap();
    let token = creds.token.unwrap();
    let secret = creds.webhook_secret.expect("secret returned once");
    let ch = fx.channel(&[creds.agent.member.id]).await;
    fx.svc().post_message(&fx.human, None, ch.id, "@hooky 你好", None).await.unwrap();

    let (headers, body) = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await.unwrap().unwrap();
    let ts: i64 = headers["x-crewconvo-timestamp"].to_str().unwrap().parse().unwrap();
    let sig = headers["x-crewconvo-signature"].to_str().unwrap();
    assert!(ac_server::webhook::verify(&secret, ts, &body, sig, chrono::Utc::now().timestamp(), 300));
    assert!(!ac_server::webhook::verify("wrong-secret", ts, &body, sig, chrono::Utc::now().timestamp(), 300));
    let ev: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(ev["type"], "wake");
    let wake_id = ev["payload"]["wake_id"].as_str().unwrap().to_string();

    // 2xx counted as ack.
    wait_for("running", Duration::from_secs(5), || async {
        fx.store().get_wake(wake_id.parse().unwrap()).await.unwrap().unwrap().status == WakeStatus::Running
    })
    .await;
    let c = reqwest::Client::new();
    tool(&c, &base, &token, "post_message", json!({"conversation_id": ch.id, "body": "webhook agent 回覆"})).await;
    let r = c
        .post(format!("{base}/agent/v1/replies"))
        .bearer_auth(&token)
        .json(&json!({"type":"wake.done","wake_id":wake_id,"private_log":"ok"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    wait_for("wake done with one post", Duration::from_secs(10), || async {
        let w = fx.store().get_wake(wake_id.parse().unwrap()).await.unwrap().unwrap();
        (w.status, w.posted_count) == (WakeStatus::Done, 1)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokens_rotate_and_revoke() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let creds = fx.svc().create_external_agent(&fx.human, ext_input("rotor", "websocket", None)).await.unwrap();
    let old = creds.token.unwrap();
    let id = creds.agent.member.id;
    let c = reqwest::Client::new();
    let me = |t: String| c.get(format!("{base}/agent/v1/me")).bearer_auth(t).send();
    assert_eq!(me(old.clone()).await.unwrap().status(), 200);
    let rotated = fx.svc().rotate_agent_token(&fx.human, id).await.unwrap().token.unwrap();
    assert_eq!(me(old).await.unwrap().status(), 401);
    assert_eq!(me(rotated.clone()).await.unwrap().status(), 200);
    fx.svc().revoke_agent_token(&fx.human, id).await.unwrap();
    assert_eq!(me(rotated).await.unwrap().status(), 401);
    // Only token hashes are stored.
    let (stored,): (Option<String>,) = sqlx::query_as("SELECT token_hash FROM agents WHERE member_id = $1")
        .bind(id)
        .fetch_one(&fx.store().pool)
        .await
        .unwrap();
    assert!(stored.is_none());
    // A different owner cannot manage the agent.
    let bob = fx.second_human("bob", false).await;
    assert!(fx.svc().rotate_agent_token(&bob, id).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn builtin_and_external_agents_share_one_action_path() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let creds = fx.svc().create_external_agent(&fx.human, ext_input("outsider", "websocket", None)).await.unwrap();
    let (_, builtin_m) = fx.agent("insider", "mock:silent").await;
    let private = fx
        .svc()
        .create_channel(
            &fx.human,
            &format!("p-{}", uuid::Uuid::now_v7().simple()),
            None,
            ac_core::model::Visibility::Private,
            &[],
        )
        .await
        .unwrap();
    // Same rule, same error, whichever transport.
    let c = reqwest::Client::new();
    let r = c
        .post(format!("{base}/agent/v1/tools/post_message"))
        .bearer_auth(creds.token.unwrap())
        .json(&json!({"conversation_id": private.id, "body": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let e = fx
        .svc()
        .call_tool(&builtin_m, None, "post_message", json!({"conversation_id": private.id, "body": "x"}))
        .await
        .unwrap_err();
    assert!(matches!(e, ac_core::AppError::Forbidden(_)));
}

/// Needs access to the Docker socket: `CREWCONVO_SANDBOX_TESTS=1` (run the test binary with a user
/// that can reach Docker).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sandboxes_are_isolated_per_agent() {
    if std::env::var("CREWCONVO_SANDBOX_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set CREWCONVO_SANDBOX_TESTS=1");
        return;
    }
    let sb = ac_sandbox::DockerSandbox::new(ac_sandbox::DockerSandboxConfig::default());
    assert!(ac_sandbox::Sandbox::available(sb.as_ref()).await, "docker not reachable");
    let fx =
        fixture_with(Opts { sandbox: Some(sb.clone() as Arc<dyn ac_sandbox::Sandbox>), ..Default::default() }).await;
    let (a, am) = fx.agent("boxa", "mock:silent").await;
    let (b, bm) = fx.agent("boxb", "mock:silent").await;
    let call = |m: &ac_core::model::Member, name: &str, args: Value| {
        let svc = fx.svc().clone();
        let m = m.clone();
        let name = name.to_string();
        async move { svc.call_tool(&m, None, &name, args).await }
    };

    call(&am, "write_file", json!({"path": "memory/secret.txt", "content": "A 的秘密"})).await.unwrap();
    let r = call(&am, "read_file", json!({"path": "/workspace/memory/secret.txt"})).await.unwrap();
    assert_eq!(r["content"], "A 的秘密");
    // B has its own empty workspace.
    assert!(call(&bm, "read_file", json!({"path": "memory/secret.txt"})).await.is_err());
    let out = call(
        &bm,
        "bash",
        json!({"command": "ls -la /workspace; id -u; cat /proc/1/cgroup >/dev/null; mount | grep workspace"}),
    )
    .await
    .unwrap();
    let stdout = out["stdout"].as_str().unwrap();
    assert!(!stdout.contains("secret"), "{stdout}");
    assert!(stdout.contains("1000"), "runs as uid 1000: {stdout}");
    assert!(stdout.contains(&b.member.id.simple().to_string()) || !stdout.contains(&a.member.id.simple().to_string()));
    // Path traversal is rejected before reaching the container.
    assert!(call(&bm, "read_file", json!({"path": "../etc/passwd"})).await.is_err());
    // Hardening: read-only root, no network, no privilege escalation.
    let out = call(&bm, "bash", json!({"command": "touch /etc/x 2>&1; python3 -c \"import socket; socket.create_connection(('1.1.1.1', 53), 3)\" 2>&1 | tail -1; cat /proc/self/status | grep -i -E '^(CapEff|NoNewPrivs)'"})).await.unwrap();
    let s = out["stdout"].as_str().unwrap();
    assert!(s.contains("Read-only file system"), "{s}");
    assert!(s.contains("Network is unreachable") || s.contains("OSError") || s.contains("Errno"), "{s}");
    assert!(s.contains("CapEff:\t0000000000000000"), "{s}");
    assert!(s.contains("NoNewPrivs:\t1"), "{s}");
    // External agents have no sandbox tools.
    let ext = fx.svc().create_external_agent(&fx.human, ext_input("nobox", "websocket", None)).await.unwrap();
    assert!(call(&ext.agent.member, "bash", json!({"command": "id"})).await.is_err());
    // Deleting an agent destroys its workspace (confirm required).
    assert!(fx.svc().delete_agent(&fx.human, a.member.id, false).await.is_err());
    fx.svc().delete_agent(&fx.human, a.member.id, true).await.unwrap();
    fx.svc().delete_agent(&fx.human, b.member.id, true).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_poll_agent_activity_feed_and_agent_view() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let creds = fx.svc().create_external_agent(&fx.human, ext_input("poller", "websocket", None)).await.unwrap();
    let token = creds.token.unwrap();
    let ch = fx.channel(&[creds.agent.member.id]).await;
    let c = reqwest::Client::new();

    // First poll connects (agent goes online) and times out with no events.
    let r: Value = c
        .get(format!("{base}/agent/v1/events?timeout=1"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["events"].as_array().unwrap().len(), 0);
    assert_eq!(fx.store().get_agent(creds.agent.member.id).await.unwrap().unwrap().status, AgentStatus::Online);

    // A poll waiting while the agent gets tagged returns the wake.
    let poll = {
        let c = c.clone();
        let base = base.clone();
        let token = token.clone();
        tokio::spawn(async move {
            c.get(format!("{base}/agent/v1/events?timeout=20"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let trigger = fx.svc().post_message(&fx.human, None, ch.id, "@poller 你在嗎？", None).await.unwrap().message;
    let r = tokio::time::timeout(Duration::from_secs(20), poll).await.unwrap().unwrap();
    let ev = &r["events"][0];
    assert_eq!(ev["type"], "wake", "{r}");
    let wake_id = ev["payload"]["wake_id"].as_str().unwrap().to_string();
    for reply in [
        json!({"type":"wake.ack","wake_id":wake_id}),
        json!({"type":"wake.done","wake_id":wake_id,"private_log":"long-poll ok"}),
    ] {
        assert!(
            c.post(format!("{base}/agent/v1/replies"))
                .bearer_auth(&token)
                .json(&reply)
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );
    }
    wait_for("done", Duration::from_secs(10), || async {
        fx.store().get_wake(wake_id.parse().unwrap()).await.unwrap().unwrap().status == WakeStatus::Done
    })
    .await;

    // Activity feed and the exact payload the agent saw.
    let feed = fx.svc().activity(&fx.human, None, Some(10)).await.unwrap();
    assert!(feed.iter().any(|a| a.wake.id.to_string() == wake_id && a.agent_handle == "poller"));
    let payload = fx.svc().wake_payload(&fx.human, wake_id.parse().unwrap()).await.unwrap();
    assert_eq!(payload["trigger_message_id"], trigger.id.to_string());
    let outsider = fx.second_human("eve", false).await;
    assert!(fx.svc().wake_payload(&outsider, wake_id.parse().unwrap()).await.is_err(), "non-members cannot see it");

    // Search operators.
    fx.svc().post_message(&fx.human, None, ch.id, "部署完成，版本 v9", None).await.unwrap();
    let p = ac_search::SearchParams {
        query: format!("from:@alice in:#{} 部署", ch.name.clone().unwrap()),
        ..Default::default()
    };
    wait_for("operator search", Duration::from_secs(10), || async {
        fx.svc().search().search(&fx.human, &p).await.map(|r| r.total == 1).unwrap_or(false)
    })
    .await;
    let none = ac_search::SearchParams { query: "from:@poller 部署".into(), ..Default::default() };
    assert_eq!(fx.svc().search().search(&fx.human, &none).await.unwrap().total, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn browser_connect_flow_and_agent_channel_visibility() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let c = reqwest::Client::new();
    fx.svc().create_channel(&fx.human, "general", Some("大廳"), ac_core::model::Visibility::Public, &[]).await.unwrap();

    // `crewconvo connect`: the CLI starts a request, a signed-in human approves it in the browser,
    // the CLI collects the token exactly once.
    let start = |c: reqwest::Client, base: String| async move {
        let r: Value = c
            .post(format!("{base}/agent/v1/connect/start"))
            .json(&json!({"client": {"hostname": "laptop", "os": "Linux"}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        r
    };
    let poll = |c: reqwest::Client, base: String, device: String| async move {
        c.post(format!("{base}/agent/v1/connect/poll")).json(&json!({"device_code": device})).send().await.unwrap()
    };
    let s1 = start(c.clone(), base.clone()).await;
    let device = s1["device_code"].as_str().unwrap().to_string();
    let user_code = s1["user_code"].as_str().unwrap().to_string();
    assert_eq!(user_code.len(), 9, "XXXX-XXXX: {s1}");
    assert!(s1["verification_url_complete"].as_str().unwrap().ends_with(&format!("/connect?code={user_code}")));
    let p: Value = poll(c.clone(), base.clone(), device.clone()).await.json().await.unwrap();
    assert_eq!(p["status"], "pending");
    // The approver sees which machine is asking; lowercase / no dash is accepted.
    let view = fx.svc().connect_request(&fx.human, &user_code.replace('-', "").to_lowercase()).await.unwrap();
    assert_eq!(view.client["hostname"], "laptop");
    let approve = ac_service::connect::ApproveInput {
        handle: Some("remote-bot".into()),
        display_name: Some("Remote".into()),
        description: Some("其他平台來的 Agent".into()),
        expertise: vec!["整理".into()],
        ..Default::default()
    };
    let agent = fx.svc().connect_approve(&fx.human, &user_code, approve.clone()).await.unwrap();
    assert_eq!(agent.member.handle, "remote-bot");
    assert!(fx.svc().connect_approve(&fx.human, &user_code, approve).await.is_err(), "a code is approved once");
    let p: Value = poll(c.clone(), base.clone(), device.clone()).await.json().await.unwrap();
    assert_eq!(p["status"], "approved");
    assert_eq!(p["handle"], "remote-bot");
    let token = p["token"].as_str().expect("token").to_string();
    assert_eq!(poll(c.clone(), base.clone(), device.clone()).await.status(), 401, "the token is handed out once");
    let me: Value =
        c.get(format!("{base}/agent/v1/me")).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
    assert_eq!(me["member"]["handle"], "remote-bot");
    let r = json!({"agent": {"member": {"id": agent.member.id.to_string()}}});
    // Denied requests never yield a token.
    let s2 = start(c.clone(), base.clone()).await;
    fx.svc().connect_deny(&fx.human, s2["user_code"].as_str().unwrap()).await.unwrap();
    let p: Value =
        poll(c.clone(), base.clone(), s2["device_code"].as_str().unwrap().to_string()).await.json().await.unwrap();
    assert_eq!(p["status"], "denied");
    assert!(p.get("token").is_none());
    // Reconnecting an existing agent rotates its token (the old one stops working).
    let s3 = start(c.clone(), base.clone()).await;
    fx.svc()
        .connect_approve(
            &fx.human,
            s3["user_code"].as_str().unwrap(),
            ac_service::connect::ApproveInput { agent_id: Some(agent.member.id), ..Default::default() },
        )
        .await
        .unwrap();
    let p: Value =
        poll(c.clone(), base.clone(), s3["device_code"].as_str().unwrap().to_string()).await.json().await.unwrap();
    let token2 = p["token"].as_str().unwrap().to_string();
    assert_eq!(c.get(format!("{base}/agent/v1/me")).bearer_auth(&token).send().await.unwrap().status(), 401);
    let token = token2;
    let bob = fx.second_human("bob", false).await;

    // Channel visibility for agents: only their own channels plus the #general lobby.
    let (helper, _) = fx.agent("helper", "mock:silent").await;
    let agent_id: uuid::Uuid = r["agent"]["member"]["id"].as_str().unwrap().parse().unwrap();
    let ch = fx
        .svc()
        .create_channel(
            &fx.human,
            "release-room",
            Some("上線協調"),
            ac_core::model::Visibility::Public,
            &[helper.member.id],
        )
        .await
        .unwrap();
    let names = |d: &Value| {
        d["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };
    let dir = tool(&c, &base, &token, "list_channels", json!({})).await;
    assert_eq!(names(&dir), vec!["general".to_string()], "agent sees only the lobby: {dir}");
    assert_eq!(dir["channels"][0]["joined"], false, "a new agent starts in no channel");
    let r2 = c
        .post(format!("{base}/agent/v1/tools/join_channel"))
        .bearer_auth(&token)
        .json(&json!({"channel": "#release-room"}))
        .send()
        .await
        .unwrap();
    assert!(!r2.status().is_success(), "agents cannot join other public channels on their own");
    let r2 = c
        .post(format!("{base}/agent/v1/tools/join_channel"))
        .bearer_auth(&token)
        .json(&json!({"channel": ch.id}))
        .send()
        .await
        .unwrap();
    assert!(!r2.status().is_success(), "not by id either");
    tool(&c, &base, &token, "join_channel", json!({"channel": "#general"})).await;
    // All agents stay visible.
    let agents = tool(&c, &base, &token, "list_agents", json!({})).await;
    assert!(agents["agents"].as_array().unwrap().iter().any(|a| a["handle"] == "helper"), "{agents}");
    // After an invite the channel shows up.
    fx.svc().invite(&fx.human, ch.id, &[agent_id]).await.unwrap();
    let dir = tool(&c, &base, &token, "list_channels", json!({"query": "上線"})).await;
    let entry = &dir["channels"][0];
    assert_eq!(entry["name"], "release-room");
    assert_eq!(entry["joined"], true);
    assert!(entry["agents"].as_array().unwrap().iter().any(|h| h == "helper"));
    // Humans still see every public channel.
    let human_dir = fx.svc().channel_directory(&bob, None).await.unwrap();
    assert!(human_dir.iter().any(|x| x.name.as_deref() == Some("release-room")));

    // The Skill is served for manual setups.
    let skill = c.get(format!("{base}/integrations/SKILL.md")).send().await.unwrap().text().await.unwrap();
    assert!(skill.starts_with("---\nname: crewconvo"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_groups_limit_discovery_and_mention_all() {
    let fx = fixture().await;
    let base = fx.serve().await;
    let c = reqwest::Client::new();
    let bob = fx.second_human("bob", false).await;
    // Two people share the deployment; each connects an agent.
    let a = fx.svc().create_external_agent(&fx.human, ext_input("alice-bot", "websocket", None)).await.unwrap();
    let b = fx.svc().create_external_agent(&bob, ext_input("bob-bot", "websocket", None)).await.unwrap();
    let (a2, _) = fx.agent("alice-helper", "mock:silent").await; // same owner as alice-bot
    let ta = a.token.clone().unwrap();
    let handles = |v: &Value| {
        v["agents"].as_array().unwrap().iter().map(|x| x["handle"].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };

    // Without a shared group: same-owner agents only.
    let seen = handles(&tool(&c, &base, &ta, "list_agents", json!({})).await);
    assert!(seen.contains(&"alice-helper".to_string()) && !seen.contains(&"bob-bot".to_string()), "{seen:?}");
    let r = c
        .post(format!("{base}/agent/v1/tools/open_dm"))
        .bearer_auth(&ta)
        .json(&json!({"member_ids": ["@bob-bot"]}))
        .send()
        .await
        .unwrap();
    assert!(!r.status().is_success(), "cannot reach an agent outside its groups");

    // Bob creates a group with both agents → they can find each other.
    let g = fx
        .svc()
        .create_agent_group(
            &bob,
            ac_service::groups::GroupInput {
                name: Some("合作專案".into()),
                description: None,
                agent_ids: Some(vec![a.agent.member.id, b.agent.member.id]),
            },
        )
        .await
        .unwrap();
    let seen = handles(&tool(&c, &base, &ta, "list_agents", json!({})).await);
    assert!(seen.contains(&"bob-bot".to_string()), "{seen:?}");
    tool(&c, &base, &ta, "open_dm", json!({"member_ids": ["@bob-bot"]})).await;
    // Only the creator or an admin edits it.
    let eve = fx.second_human("eve", false).await;
    assert!(
        fx.svc()
            .update_agent_group(
                &eve,
                g.id,
                ac_service::groups::GroupInput { name: Some("x".into()), description: None, agent_ids: None }
            )
            .await
            .is_err()
    );
    fx.svc()
        .update_agent_group(
            &fx.human,
            g.id,
            ac_service::groups::GroupInput { name: None, description: None, agent_ids: Some(vec![a.agent.member.id]) },
        )
        .await
        .unwrap();
    let seen = handles(&tool(&c, &base, &ta, "list_agents", json!({})).await);
    assert!(!seen.contains(&"bob-bot".to_string()), "removed from the group: {seen:?}");

    // `all` is reserved; @all mentions everyone in the conversation (humans only).
    assert!(fx.svc().create_external_agent(&fx.human, ext_input("all", "websocket", None)).await.is_err());
    let ch = fx
        .svc()
        .create_channel(&fx.human, "allhands", None, ac_core::model::Visibility::Public, &[a2.member.id, bob.id])
        .await
        .unwrap();
    let posted = fx.svc().post_message(&fx.human, None, ch.id, "@all 早安", None).await.unwrap();
    let mut m = posted.message.mentions.clone();
    m.sort();
    assert_eq!(m, vec!["alice-helper".to_string(), "bob".to_string()]);
    assert!(posted.ignored_mentions.is_empty());
    let agent_member = fx.svc().store().get_member(a2.member.id).await.unwrap().unwrap();
    let by_agent = fx.svc().post_message(&agent_member, None, ch.id, "@all 我也要", None).await.unwrap();
    assert!(by_agent.message.mentions.is_empty());
    assert_eq!(by_agent.ignored_mentions, vec!["all".to_string()]);
}
