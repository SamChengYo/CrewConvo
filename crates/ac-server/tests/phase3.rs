//! Phase 3 acceptance tests (spec 13): Decider and multi-agent modes. A local fake
//! `decisions_api` endpoint makes the Decider deterministic.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ac_core::model::{DeciderProtocol, DiscussionStatus, MemberKind, WakeMode, WakeReason, WakeStatus};
use ac_service::deciders::DeciderConfigInput;
use ac_store::MessageScope;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use common::{Fx, fixture, settled, wait_for};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Default)]
struct FakeState {
    scores: Arc<Mutex<HashMap<String, f64>>>,
    interaction: Arc<Mutex<(String, f64)>>,
    fail: Arc<Mutex<Option<u16>>>,
    calls: Arc<Mutex<Vec<Value>>>,
}

async fn fake_handler(
    State(s): State<FakeState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut call = req.clone();
    call["_key"] = json!(headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()).unwrap_or_default());
    s.calls.lock().unwrap().push(call);
    if let Some(code) = *s.fail.lock().unwrap() {
        return (StatusCode::from_u16(code).unwrap(), Json(json!({"error":{"code":"unavailable","message":"down"}})));
    }
    let scores = s.scores.lock().unwrap().clone();
    let (choice, conf) = s.interaction.lock().unwrap().clone();
    let mut answers = serde_json::Map::new();
    for (k, q) in req["questions"].as_object().unwrap() {
        if k == "interaction" {
            answers.insert(k.clone(), json!({"type":"choice","choice":choice,"confidence":conf}));
            continue;
        }
        let instr = q["instructions"].as_str().unwrap_or_default();
        let handle = instr.split("（@").nth(1).and_then(|r| r.split('）').next()).unwrap_or_default();
        let v = scores.get(handle).copied().unwrap_or(0.05);
        answers.insert(k.clone(), json!({"type":"noul","noul":v,"confidence":v}));
    }
    (StatusCode::OK, Json(json!({"answers": answers, "usage": {"total_tokens": 10}})))
}

async fn fake_decider(scores: &[(&str, f64)], interaction: &str, conf: f64) -> (String, FakeState) {
    let st = FakeState::default();
    *st.scores.lock().unwrap() = scores.iter().map(|(h, v)| (h.to_string(), *v)).collect();
    *st.interaction.lock().unwrap() = (interaction.to_string(), conf);
    let app = Router::new().route("/v1beta/decisions", post(fake_handler)).with_state(st.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/v1beta/decisions"), st)
}

fn cfg(name: &str, url: &str, key: &str, priority: i32) -> DeciderConfigInput {
    DeciderConfigInput {
        name: Some(name.into()),
        protocol: Some(DeciderProtocol::DecisionsApi),
        endpoint_url: Some(url.into()),
        api_key: Some(key.into()),
        model: Some("gemini-decision-1".into()),
        timeout_ms: Some(3000),
        t_wake: Some(0.6),
        t_discuss: Some(0.7),
        max_wake: Some(4),
        priority: Some(priority),
        ..Default::default()
    }
}

async fn wakes_of(fx: &Fx, msg: Uuid) -> Vec<ac_core::model::Wake> {
    fx.store().wakes_for_trigger(msg).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn falls_back_to_next_decider_and_never_blocks_writes() {
    let fx = fixture().await;
    let (alpha, _) = fx.agent_with("alpha", "mock:react", "資料庫效能", vec!["PostgreSQL"]).await;
    let (beta, _) = fx.agent_with("beta", "mock:react", "UI 設計", vec!["Figma"]).await;
    let ch = fx.channel(&[alpha.member.id, beta.member.id]).await;
    let (down_url, down) = fake_decider(&[], "independent", 0.5).await;
    *down.fail.lock().unwrap() = Some(503);
    let (ok_url, ok) = fake_decider(&[("alpha", 0.92), ("beta", 0.1)], "independent", 0.8).await;
    fx.svc().create_decider_config(&fx.human, cfg("primary (down)", &down_url, "key-primary-0001", 0)).await.unwrap();
    fx.svc().create_decider_config(&fx.human, cfg("fallback", &ok_url, "key-fallback-0002", 1)).await.unwrap();

    let started = std::time::Instant::now();
    let msg = fx.svc().post_message(&fx.human, None, ch.id, "訂單查詢很慢，可能少了索引", None).await.unwrap().message;
    assert!(started.elapsed() < Duration::from_millis(1500), "writing the message does not wait for the Decider");

    wait_for("alpha woken by fallback decider", Duration::from_secs(20), || async {
        wakes_of(&fx, msg.id).await.iter().any(|w| w.agent_id == alpha.member.id && w.status == WakeStatus::Done)
    })
    .await;
    let wakes = wakes_of(&fx, msg.id).await;
    assert_eq!(wakes.len(), 1);
    assert_eq!(wakes[0].reason, WakeReason::Decider);
    let d = fx.store().decisions_for_message(msg.id).await.unwrap();
    assert_eq!(d.len(), 1);
    let attempts = d[0].attempts.as_array().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0]["ok"], false);
    assert_eq!(attempts[1]["ok"], true);
    assert_eq!(down.calls.lock().unwrap().len(), 1);
    assert_eq!(
        ok.calls.lock().unwrap()[0]["_key"],
        "key-fallback-0002",
        "decrypted key is sent in the configured header"
    );
    // Authentication never ends up in the stored request.
    assert!(!d[0].request.as_ref().unwrap().to_string().contains("key-fallback"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_deciders_failing_wakes_nobody() {
    let fx = fixture().await;
    let (alpha, _) = fx.agent("alpha", "mock:react").await;
    let ch = fx.channel(&[alpha.member.id]).await;
    let (url, st) = fake_decider(&[], "independent", 0.5).await;
    *st.fail.lock().unwrap() = Some(500);
    fx.svc().create_decider_config(&fx.human, cfg("only", &url, "k-0000", 0)).await.unwrap();
    let msg = fx.svc().post_message(&fx.human, None, ch.id, "有人在嗎", None).await.unwrap().message;
    wait_for("decision recorded", Duration::from_secs(10), || async {
        !fx.store().decisions_for_message(msg.id).await.unwrap().is_empty()
    })
    .await;
    let d = &fx.store().decisions_for_message(msg.id).await.unwrap()[0];
    assert_eq!(d.reason.as_deref(), Some("all_failed"));
    assert!(d.woken.is_empty());
    assert!(wakes_of(&fx, msg.id).await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_changes_apply_immediately_and_keys_are_never_returned() {
    let fx = fixture().await;
    let (alpha, _) = fx.agent("alpha", "mock:react").await;
    let (beta, _) = fx.agent("beta", "mock:react").await;
    let ch = fx.channel(&[alpha.member.id, beta.member.id]).await;
    let (url1, _) = fake_decider(&[("alpha", 0.9)], "independent", 0.5).await;
    let (url2, st2) = fake_decider(&[("beta", 0.9)], "independent", 0.5).await;
    let created =
        fx.svc().create_decider_config(&fx.human, cfg("main", &url1, "sk-first-secret-AAAA", 0)).await.unwrap();
    assert!(created.has_api_key);
    assert_eq!(created.api_key_last4.as_deref(), Some("AAAA"));

    let m1 = fx.svc().post_message(&fx.human, None, ch.id, "第一個問題", None).await.unwrap().message;
    wait_for("alpha woken", Duration::from_secs(10), || async {
        wakes_of(&fx, m1.id).await.iter().any(|w| w.agent_id == alpha.member.id)
    })
    .await;

    // Admin switches endpoint, key and model; the very next message uses them.
    fx.svc()
        .update_decider_config(
            &fx.human,
            created.id,
            DeciderConfigInput {
                endpoint_url: Some(url2.clone()),
                api_key: Some("sk-second-secret-BBBB".into()),
                model: Some("gemini-decision-2".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m2 = fx.svc().post_message(&fx.human, None, ch.id, "第二個問題", None).await.unwrap().message;
    wait_for("beta woken", Duration::from_secs(10), || async {
        wakes_of(&fx, m2.id).await.iter().any(|w| w.agent_id == beta.member.id)
    })
    .await;
    let call = st2.calls.lock().unwrap()[0].clone();
    assert_eq!(call["model"], "gemini-decision-2");
    assert_eq!(call["_key"], "sk-second-secret-BBBB");

    // Over HTTP: keys never appear in any response.
    let base = fx.serve().await;
    let token = ac_service::auth::new_token();
    let token = token.as_str();
    fx.store()
        .create_session(
            &ac_service::auth::hash_token(token),
            fx.human.id,
            chrono::Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();
    let c = reqwest::Client::new();
    let body = c
        .get(format!("{base}/api/decider-configs"))
        .header("cookie", format!("cc_session={token}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("BBBB") && !body.contains("sk-second-secret") && !body.contains("sk-first-secret"), "{body}");
    let test = c
        .post(format!("{base}/api/decider-configs/{}/test", created.id))
        .header("cookie", format!("cc_session={token}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(test.contains("\"ok\":true") && !test.contains("secret"), "{test}");

    // Non-admins cannot read or change Decider settings.
    let bob = fx.second_human("bob", false).await;
    assert!(matches!(fx.svc().list_decider_configs(&bob).await, Err(ac_core::AppError::Forbidden(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_mode_wakes_several_agents_independently() {
    let fx = fixture().await;
    let (alpha, _) = fx.agent("alpha", "mock:react").await;
    let (beta, _) = fx.agent("beta", "mock:react").await;
    let (gamma, _) = fx.agent("gamma", "mock:react").await;
    let ch = fx.channel(&[alpha.member.id, beta.member.id, gamma.member.id]).await;
    let (url, _) = fake_decider(&[("alpha", 0.9), ("beta", 0.8), ("gamma", 0.2)], "independent", 0.9).await;
    fx.svc().create_decider_config(&fx.human, cfg("d", &url, "k-1111", 0)).await.unwrap();
    let msg = fx.svc().post_message(&fx.human, None, ch.id, "請各自看一下自己負責的部分", None).await.unwrap().message;
    wait_for("two wakes done", Duration::from_secs(15), || async {
        let w = wakes_of(&fx, msg.id).await;
        w.len() == 2 && w.iter().all(|w| w.status == WakeStatus::Done)
    })
    .await;
    let w = wakes_of(&fx, msg.id).await;
    assert!(w.iter().all(|w| w.mode == WakeMode::Parallel));
    assert!(!w.iter().any(|w| w.agent_id == gamma.member.id));
}

async fn run_discussion(fx: &Fx, a_model: &str, b_model: &str) -> (ac_core::model::Discussion, Uuid, Uuid) {
    let (alpha, _) = fx.agent("alpha", a_model).await;
    let (beta, _) = fx.agent("beta", b_model).await;
    let ch = fx.channel(&[alpha.member.id, beta.member.id]).await;
    let (url, _) = fake_decider(&[("alpha", 0.95), ("beta", 0.85)], "discuss", 0.9).await;
    fx.svc().create_decider_config(&fx.human, cfg("d", &url, "k-2222", 0)).await.unwrap();
    let msg = fx
        .svc()
        .post_message(&fx.human, None, ch.id, "我們該用 PostgreSQL 還是 MongoDB？請討論", None)
        .await
        .unwrap()
        .message;
    wait_for("discussion finished", Duration::from_secs(60), || async {
        match fx.store().active_discussion(ch.id, msg.id).await.unwrap() {
            Some(_) => false,
            None => !fx.store().wakes_for_trigger(msg.id).await.unwrap().is_empty() && settled(fx, ch.id).await,
        }
    })
    .await;
    let wakes = fx.store().wakes_for_trigger(msg.id).await.unwrap();
    let did = wakes[0].discussion_id.expect("discussion wake");
    (fx.store().get_discussion(did).await.unwrap().unwrap(), ch.id, msg.id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discussion_ends_when_everyone_skips() {
    let fx = fixture().await;
    let (d, ch, root) = run_discussion(&fx, "mock:silent", "mock:silent").await;
    assert_eq!(d.status, DiscussionStatus::Done);
    assert_eq!(d.end_reason.as_deref(), Some("all_skipped"));
    let wakes = fx.store().wakes_for_discussion(d.id).await.unwrap();
    assert_eq!(wakes.len(), 3, "one round (2 turns) + summarizer");
    let thread = fx.store().list_messages(ch, MessageScope::Thread(root), None, 50).await.unwrap();
    assert!(thread.iter().any(|m| m.author_kind == MemberKind::System && m.body.contains("討論已結束")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discussion_runs_max_rounds_then_posts_conclusion_in_thread_and_main() {
    let fx = fixture().await;
    let (d, ch, root) = run_discussion(&fx, "mock:reply", "mock:reply").await;
    assert_eq!(d.status, DiscussionStatus::Done);
    assert_eq!(d.end_reason.as_deref(), Some("max_rounds"));
    let wakes = fx.store().wakes_for_discussion(d.id).await.unwrap();
    assert_eq!(wakes.len(), 3 * 2 + 1, "3 rounds × 2 participants + summarizer");
    // Turn order: alpha (highest score) then beta.
    let alpha = fx.store().member_by_handle(fx.workspace_id, "alpha").await.unwrap().unwrap();
    assert_eq!(wakes[0].agent_id, alpha.id);
    assert_eq!(d.summarizer_id, Some(alpha.id));
    let thread = fx.store().list_messages(ch, MessageScope::Thread(root), None, 50).await.unwrap();
    assert_eq!(thread.iter().filter(|m| m.author_kind == MemberKind::Agent).count(), 7, "6 turns + conclusion");
    assert!(thread.iter().any(|m| m.body.contains("mock 結論")));
    let main = fx.store().list_messages(ch, MessageScope::Main, None, 50).await.unwrap();
    assert!(main.iter().any(|m| m.author_kind == MemberKind::Agent && m.body.contains("同步到主對話")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thread_with_several_agents_asks_decider_among_participants() {
    let fx = fixture().await;
    let (alpha, am) = fx.agent("alpha", "mock:silent").await;
    let (beta, bm) = fx.agent("beta", "mock:silent").await;
    let (gamma, _) = fx.agent("gamma", "mock:silent").await;
    let ch = fx.channel(&[alpha.member.id, beta.member.id, gamma.member.id]).await;
    let (url, st) = fake_decider(&[("beta", 0.9), ("alpha", 0.3), ("gamma", 0.99)], "independent", 0.5).await;
    fx.svc().create_decider_config(&fx.human, cfg("d", &url, "k-3333", 0)).await.unwrap();
    let root = fx.svc().post_message(&fx.human, None, ch.id, "@alpha @beta 看一下這個", None).await.unwrap().message;
    fx.svc().post_message(&am, None, ch.id, "alpha 的意見", Some(root.id)).await.unwrap();
    fx.svc().post_message(&bm, None, ch.id, "beta 的意見", Some(root.id)).await.unwrap();
    wait_for("settled", Duration::from_secs(10), || settled(&fx, ch.id)).await;
    st.calls.lock().unwrap().clear();
    let reply = fx.svc().post_message(&fx.human, None, ch.id, "那第二點呢？", Some(root.id)).await.unwrap().message;
    wait_for("beta woken in thread", Duration::from_secs(10), || async {
        wakes_of(&fx, reply.id).await.iter().any(|w| w.agent_id == beta.member.id)
    })
    .await;
    let w = wakes_of(&fx, reply.id).await;
    assert_eq!(w.len(), 1, "gamma never spoke in the thread, so it is not a candidate even with a high score");
    assert_eq!(w[0].reason, WakeReason::ThreadReply);
    assert_eq!(w[0].thread_root_id, Some(root.id));
    let questions = st.calls.lock().unwrap()[0]["questions"].as_object().unwrap().len();
    assert_eq!(questions, 3, "2 thread participants + interaction");
}
