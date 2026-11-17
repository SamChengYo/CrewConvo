//! Phase 2 acceptance tests (spec 13): memory and search.

mod common;

use std::time::Duration;

use ac_context::{ContextAssembler, ContextBudget, payload_message_tokens};
use ac_core::model::{MemberKind, Visibility, WakeMode, WakeReason, WakeStatus};
use ac_search::SearchParams;
use ac_store::{MessageScope, NewWake};
use common::{Opts, fixture, fixture_with, wait_for};
use uuid::Uuid;

/// Inserts `n` messages in one statement (fast); message `special_at` gets `special` as body.
async fn bulk_insert(fx: &common::Fx, conv: Uuid, author: Uuid, n: i64, special_at: i64, special: &str) {
    sqlx::query(
        "WITH s AS (SELECT gen_random_uuid() AS id, g FROM generate_series(1, $3::bigint) g) \
         INSERT INTO messages (id, conversation_id, seq, author_id, body, depth, chain_root_id, created_at) \
         SELECT id, $1, g, $2, \
                CASE WHEN g = $4 THEN $5 \
                     ELSE format('第 %s 則：例行進度更新，今天完成了模組 %s 的測試、程式碼審查與文件整理，明天繼續處理剩下的項目。', g, g % 37) END, \
                0, id, now() - make_interval(secs => ($3 - g)::double precision) \
         FROM s",
    )
    .bind(conv)
    .bind(author)
    .bind(n)
    .bind(special_at)
    .bind(special)
    .execute(&fx.store().pool)
    .await
    .unwrap();
    sqlx::query("UPDATE conversations SET next_seq = $2 + 1 WHERE id = $1")
        .bind(conv)
        .bind(n)
        .execute(&fx.store().pool)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_channel_payload_stays_in_budget_and_agent_finds_early_messages() {
    let fx = fixture_with(Opts { summary_model: Some("mock:summarize".into()), ..Default::default() }).await;
    let (finder, _) = fx.agent("finder", "mock:search:紫水晶").await;
    // A second agent that never gets woken: its cursor stays at 0, so all 2000 messages are unread.
    let (reader, _) = fx.agent("reader", "mock:silent").await;
    let ch = fx.channel(&[finder.member.id, reader.member.id]).await;
    bulk_insert(&fx, ch.id, fx.human.id, 2000, 5, "專案代號「紫水晶」的預算確定是 42 萬，負責人是 @alice。").await;
    let n = fx.svc().search().reindex_conversation(ch.id).await.unwrap();
    assert_eq!(n, 2000);

    // Rolling summary covers everything older than the recent window.
    let summary = fx.app.state.summarizer.run_scope(ch.id, None).await.unwrap().expect("summary written");
    let window_start = fx.store().nth_newest_seq(ch.id, MessageScope::Main, 30).await.unwrap().unwrap();
    assert_eq!(summary.covers_up_to_seq, window_start - 1);
    assert!(summary.token_count <= 1000);

    // Payload for a wake with 2000 unread messages stays within the token budget.
    let trigger = fx.svc().post_message(&fx.human, None, ch.id, "@finder 你好", None).await.unwrap().message;
    let (wake, _) = fx
        .store()
        .create_or_merge_wake(&NewWake {
            agent_id: reader.member.id,
            conversation_id: ch.id,
            thread_root_id: None,
            trigger_message_id: trigger.id,
            reason: WakeReason::Mention,
            mode: WakeMode::Single,
            discussion_id: None,
            depth: 0,
            chain_root_id: trigger.id,
        })
        .await
        .unwrap();
    let assembler = ContextAssembler::new(fx.store().clone(), Default::default(), ContextBudget::default());
    let payload = assembler.assemble(&wake, &reader).await.unwrap();
    let tokens = payload_message_tokens(&payload);
    assert!(tokens <= 8000, "recent + unread = {tokens} tokens");
    assert!(payload.unread_omitted > 1500, "older unread messages are omitted: {}", payload.unread_omitted);
    assert!(payload.summary.as_deref().unwrap_or("").contains("摘要"));
    fx.store().finish_wake(wake.id, WakeStatus::Cancelled, None, None).await.unwrap();

    // The agent uses search_messages to find message #5, then reports it.
    wait_for("finder reports the early message", Duration::from_secs(30), || async {
        let msgs = fx.store().list_messages(ch.id, MessageScope::Main, None, 5).await.unwrap();
        msgs.iter().any(|m| m.author_kind == MemberKind::Agent && m.body.contains("42 萬"))
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_cannot_search_conversations_they_are_not_in() {
    let fx = fixture().await;
    let (agent, agent_m) = fx.agent("snoop", "mock:silent").await;
    let secret = fx
        .svc()
        .create_channel(&fx.human, &format!("secret-{}", Uuid::now_v7().simple()), None, Visibility::Private, &[])
        .await
        .unwrap();
    let public_not_joined = fx.channel(&[]).await;
    let mine = fx.channel(&[agent.member.id]).await;
    fx.svc().post_message(&fx.human, None, secret.id, "機密代號藍鯨，不要外流", None).await.unwrap();
    fx.svc().post_message(&fx.human, None, public_not_joined.id, "公開頻道也提到藍鯨", None).await.unwrap();
    fx.svc().post_message(&fx.human, None, mine.id, "這裡沒有那個詞", None).await.unwrap();

    let q = SearchParams { query: "藍鯨".into(), ..Default::default() };
    wait_for("indexed", Duration::from_secs(10), || async {
        fx.svc().search().search(&fx.human, &q).await.map(|r| r.total == 2).unwrap_or(false)
    })
    .await;

    // Through the tool, exactly like an external agent would call it.
    let r = fx.svc().call_tool(&agent_m, None, "search_messages", serde_json::json!({"query": "藍鯨"})).await.unwrap();
    assert_eq!(r["total"], 0, "{r}");
    // Explicitly targeting the private conversation is forbidden.
    let err = fx
        .svc()
        .call_tool(
            &agent_m,
            None,
            "search_messages",
            serde_json::json!({"query": "藍鯨", "conversation_id": secret.id}),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ac_core::AppError::Forbidden(_)));
    // Pagination happens after the permission filter.
    let r = fx
        .svc()
        .search()
        .search(&fx.human, &SearchParams { query: "藍鯨".into(), limit: Some(1), ..Default::default() })
        .await
        .unwrap();
    assert_eq!((r.total, r.hits.len()), (2, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chinese_two_character_words_are_searchable() {
    let fx = fixture().await;
    let ch = fx.channel(&[]).await;
    let m =
        fx.svc().post_message(&fx.human, None, ch.id, "我們明天要部署新版本到正式環境", None).await.unwrap().message;
    fx.svc().post_message(&fx.human, None, ch.id, "午餐吃什麼", None).await.unwrap();
    for q in ["部署", "版本", "正式環境", "部署 版本", "\"部署新版本\""] {
        let p = SearchParams { query: q.into(), conversation_id: Some(ch.id), ..Default::default() };
        wait_for(&format!("hit for {q}"), Duration::from_secs(10), || async {
            fx.svc()
                .search()
                .search(&fx.human, &p)
                .await
                .map(|r| r.hits.first().map(|h| h.message.id) == Some(m.id))
                .unwrap_or(false)
        })
        .await;
    }
    // Context and deletions.
    let r = fx
        .svc()
        .search()
        .search(&fx.human, &SearchParams { query: "部署".into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(r.hits[0].after.as_ref().map(|a| a.body.as_str()), Some("午餐吃什麼"));
    fx.svc().delete_message(&fx.human, m.id).await.unwrap();
    wait_for("deleted message disappears", Duration::from_secs(10), || async {
        fx.svc()
            .search()
            .search(&fx.human, &SearchParams { query: "部署".into(), ..Default::default() })
            .await
            .map(|r| r.total == 0)
            .unwrap_or(false)
    })
    .await;
}
