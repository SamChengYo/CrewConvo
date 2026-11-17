//! Phase 1 acceptance tests (spec 13): chatting without runaway loops.

mod common;

use std::time::Duration;

use ac_core::AppError;
use ac_core::model::{MemberKind, WakeStatus};
use ac_store::MessageScope;
use common::{fixture, settled, wait_for};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dm_between_two_agents_stops_at_max_depth() {
    let fx = fixture().await;
    let (_a, am) = fx.agent("ping", "mock:reply").await;
    let (b, _bm) = fx.agent("pong", "mock:reply").await;
    let dm = fx.svc().open_dm(&am, &[b.member.id]).await.unwrap();

    // Agent A starts the conversation proactively (depth 0, own chain).
    let first = fx.svc().post_message(&am, None, dm.id, "嗨，來聊聊吧", None).await.unwrap().message;

    let max_depth = fx.svc().limits().max_depth;
    wait_for("system notice about the depth limit", Duration::from_secs(60), || async {
        let msgs = fx.store().list_messages(dm.id, MessageScope::Main, None, 100).await.unwrap();
        msgs.iter().any(|m| m.author_kind == MemberKind::System && m.body.contains("上限"))
    })
    .await;
    wait_for("chain settles", Duration::from_secs(30), || settled(&fx, dm.id)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let msgs = fx.store().list_messages(dm.id, MessageScope::Main, None, 100).await.unwrap();
    let agent_msgs: Vec<_> = msgs.iter().filter(|m| m.author_kind == MemberKind::Agent).collect();
    let depths: Vec<i32> = agent_msgs.iter().map(|m| m.depth).collect();
    assert_eq!(depths, (0..=max_depth).collect::<Vec<_>>(), "one message per depth 0..=MAX_DEPTH");
    assert!(agent_msgs.iter().all(|m| m.chain_root_id == first.id));
    let notices = msgs.iter().filter(|m| m.author_kind == MemberKind::System).count();
    assert_eq!(notices, 1, "exactly one limit notice");

    let wakes = fx.store().wakes_for_chain(first.id).await.unwrap();
    assert_eq!(wakes.len() as i32, max_depth, "messages at depth < MAX_DEPTH each woke the other agent once");
    assert!(wakes.iter().all(|w| w.status == WakeStatus::Done && w.posted_count == 1));
    assert!(wakes.iter().all(|w| w.private_log.is_some()), "final text goes to private_log");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tagged_agent_can_react_without_speaking() {
    let fx = fixture().await;
    let (r, _) = fx.agent("acker", "mock:react").await;
    let ch = fx.channel(&[r.member.id]).await;

    let posted = fx.svc().post_message(&fx.human, None, ch.id, "@acker 報告已上傳", None).await.unwrap();
    assert!(posted.ignored_mentions.is_empty());
    let msg = posted.message;

    wait_for("wake done", Duration::from_secs(20), || async {
        let w = fx.store().wakes_for_trigger(msg.id).await.unwrap();
        w.len() == 1 && w[0].status == WakeStatus::Done
    })
    .await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(settled(&fx, ch.id).await);

    let view = fx.store().get_message_view(msg.id).await.unwrap().unwrap();
    assert_eq!(view.reactions.len(), 1);
    assert_eq!(view.reactions[0].handles, vec!["acker".to_string()]);

    let msgs = fx.store().list_messages(ch.id, MessageScope::Main, None, 100).await.unwrap();
    assert!(msgs.iter().all(|m| m.author_kind != MemberKind::Agent), "agent did not post");
    let wakes = fx.store().wakes_for_chain(msg.id).await.unwrap();
    assert_eq!(wakes.len(), 1, "a reaction triggers no further wake");
    assert_eq!(wakes[0].posted_count, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mention_of_non_member_is_ignored_with_notice() {
    let fx = fixture().await;
    let (outsider, _) = fx.agent("outsider", "mock:reply").await;
    let ch = fx.channel(&[]).await;
    let r = fx.svc().post_message(&fx.human, None, ch.id, "@outsider 在嗎？", None).await.unwrap();
    assert_eq!(r.ignored_mentions, vec!["outsider".to_string()]);
    assert!(r.message.mentions.is_empty());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(fx.store().wakes_for_trigger(r.message.id).await.unwrap().is_empty());
    let msgs = fx.store().list_messages(ch.id, MessageScope::Main, None, 10).await.unwrap();
    assert!(msgs.iter().any(|m| m.author_kind == MemberKind::System && m.body.contains("@outsider")));
    let _ = outsider;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_cancels_wakes_and_freezes_until_a_human_speaks() {
    let fx = fixture().await;
    let (slow, slow_m) = fx.agent("sloth", "mock:slow").await;
    let (helper, helper_m) = fx.agent("helper", "mock:reply").await;
    let ch = fx.channel(&[slow.member.id, helper.member.id]).await;

    let msg = fx.svc().post_message(&fx.human, None, ch.id, "@sloth 慢慢來", None).await.unwrap().message;
    wait_for("wake running", Duration::from_secs(20), || async {
        let w = fx.store().wakes_for_trigger(msg.id).await.unwrap();
        w.len() == 1 && w[0].status == WakeStatus::Running
    })
    .await;
    let wake_id = fx.store().wakes_for_trigger(msg.id).await.unwrap()[0].id;

    let cancelled = fx.svc().stop_conversation(&fx.human, ch.id).await.unwrap();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(fx.store().get_wake(wake_id).await.unwrap().unwrap().status, WakeStatus::Cancelled);
    assert!(fx.store().get_conversation(ch.id).await.unwrap().unwrap().frozen_until_human);

    // The cancelled wake can no longer be used to speak.
    let err = fx.svc().post_message(&slow_m, Some(wake_id), ch.id, "我還是想說", None).await.unwrap_err();
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");

    // Agent messages no longer wake anybody.
    let agent_msg = fx.svc().post_message(&helper_m, None, ch.id, "@sloth 你還好嗎", None).await.unwrap().message;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(fx.store().wakes_for_trigger(agent_msg.id).await.unwrap().is_empty());

    // A human message unfreezes and wakes again.
    let human_msg = fx.svc().post_message(&fx.human, None, ch.id, "@helper 繼續吧", None).await.unwrap().message;
    wait_for("helper wake done", Duration::from_secs(20), || async {
        let w = fx.store().wakes_for_trigger(human_msg.id).await.unwrap();
        w.len() == 1 && w[0].status == WakeStatus::Done
    })
    .await;
    assert!(!fx.store().get_conversation(ch.id).await.unwrap().unwrap().frozen_until_human);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn silent_agent_keeps_private_log_and_cursor_advances() {
    let fx = fixture().await;
    let (quiet, _) = fx.agent("quiet", "mock:silent").await;
    let ch = fx.channel(&[quiet.member.id]).await;
    fx.svc().post_message(&fx.human, None, ch.id, "先說一句沒 tag 的", None).await.unwrap();
    let msg = fx.svc().post_message(&fx.human, None, ch.id, "@quiet 看一下", None).await.unwrap().message;
    wait_for("wake done", Duration::from_secs(20), || async {
        let w = fx.store().wakes_for_trigger(msg.id).await.unwrap();
        w.len() == 1 && w[0].status == WakeStatus::Done
    })
    .await;
    let w = &fx.store().wakes_for_trigger(msg.id).await.unwrap()[0];
    assert_eq!(w.posted_count, 0);
    assert!(w.private_log.as_deref().unwrap_or("").contains("不需要回覆"));
    assert!(w.tokens_used > 0);
    assert_eq!(fx.store().get_cursor(ch.id, quiet.member.id).await.unwrap(), msg.seq);
    let msgs = fx.store().list_messages(ch.id, MessageScope::Main, None, 10).await.unwrap();
    assert!(msgs.iter().all(|m| m.author_kind != MemberKind::Agent));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unresponsive_agent_times_out() {
    let fx = fixture().await;
    let (slow, _) = fx.agent("snail", "mock:slow").await;
    fx.store()
        .update_agent(slow.member.id, &ac_store::AgentPatch { max_wake_seconds: Some(2), ..Default::default() })
        .await
        .unwrap();
    let ch = fx.channel(&[slow.member.id]).await;
    let msg = fx.svc().post_message(&fx.human, None, ch.id, "@snail 嗨", None).await.unwrap().message;
    wait_for("wake timeout", Duration::from_secs(20), || async {
        let w = fx.store().wakes_for_trigger(msg.id).await.unwrap();
        w.len() == 1 && w[0].status == WakeStatus::Timeout
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_api_smoke() {
    let fx = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = fx.app.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://{addr}");
    let c = reqwest::Client::builder().cookie_store(true).build().unwrap();

    let handle = format!("u{}", &Uuid::now_v7().simple().to_string()[20..]);
    let r = c
        .post(format!("{base}/api/auth/register"))
        .json(&serde_json::json!({"handle": handle, "display_name": "測試者", "password": "password123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let me: serde_json::Value = c.get(format!("{base}/api/me")).send().await.unwrap().json().await.unwrap();
    assert_eq!(me["handle"], handle);

    let ch: serde_json::Value = c
        .post(format!("{base}/api/conversations"))
        .json(&serde_json::json!({"name": format!("http-{}", Uuid::now_v7().simple()), "visibility": "public"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap();
    let posted = c
        .post(format!("{base}/api/conversations/{cid}/messages"))
        .json(&serde_json::json!({"body": "哈囉 HTTP"}))
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), 200);
    let msgs: Vec<serde_json::Value> =
        c.get(format!("{base}/api/conversations/{cid}/messages")).send().await.unwrap().json().await.unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["body"], "哈囉 HTTP");

    let anon = reqwest::Client::new().get(format!("{base}/api/me")).send().await.unwrap();
    assert_eq!(anon.status(), 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thread_reply_wakes_agent_tagged_in_root_even_if_it_answered_in_main() {
    let fx = fixture().await;
    let (a, _) = fx.agent("rooted", "mock:silent").await;
    let ch = fx.channel(&[a.member.id]).await;
    let root = fx.svc().post_message(&fx.human, None, ch.id, "@rooted 第一個問題", None).await.unwrap().message;
    wait_for("settled", Duration::from_secs(10), || settled(&fx, ch.id)).await;
    let reply = fx.svc().post_message(&fx.human, None, ch.id, "追問細節", Some(root.id)).await.unwrap().message;
    wait_for("woken in thread", Duration::from_secs(10), || async {
        fx.store().wakes_for_trigger(reply.id).await.unwrap().iter().any(|w| w.agent_id == a.member.id)
    })
    .await;
    let w = &fx.store().wakes_for_trigger(reply.id).await.unwrap()[0];
    assert_eq!(w.reason, ac_core::model::WakeReason::ThreadReply);
    assert_eq!(w.thread_root_id, Some(root.id));
}
