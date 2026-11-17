//! End-to-end test with real models (Claude Sonnet 5.5, GPT-6 Luna, Gemini 4 Hydrogen), the real
//! Gemini Decisions API and (optionally) the Docker sandbox. Costs money, so it only runs with
//! `CREWCONVO_REAL_LLM=1` (sandbox part additionally needs `CREWCONVO_SANDBOX_TESTS=1`).

mod common;

use std::sync::Arc;
use std::time::Duration;

use ac_core::model::{DeciderProtocol, DiscussionStatus, MemberKind, WakeMode, WakeReason, WakeStatus};
use ac_service::deciders::DeciderConfigInput;
use ac_store::MessageScope;
use common::{Fx, Opts, fixture_with, settled, wait_for};
use uuid::Uuid;

fn enabled() -> bool {
    common::load_env();
    std::env::var("CREWCONVO_REAL_LLM").ok().as_deref() == Some("1")
}

async fn agent_reply(fx: &Fx, conv: Uuid, agent: Uuid, after_seq: i64, timeout: u64) -> String {
    // Agents may answer in the main timeline or in the question's thread.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT body FROM messages WHERE conversation_id = $1 AND author_id = $2 AND seq > $3 \
             AND deleted_at IS NULL ORDER BY seq LIMIT 1",
        )
        .bind(conv)
        .bind(agent)
        .bind(after_seq)
        .fetch_optional(&fx.store().pool)
        .await
        .unwrap();
        if let Some((body,)) = row {
            return body;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for a reply");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn real_models_decider_discussion_and_sandbox() {
    if !enabled() {
        eprintln!("skipped: set CREWCONVO_REAL_LLM=1");
        return;
    }
    let sandbox_on = std::env::var("CREWCONVO_SANDBOX_TESTS").ok().as_deref() == Some("1");
    let sandbox = if sandbox_on {
        Some(ac_sandbox::DockerSandbox::new(ac_sandbox::DockerSandboxConfig::default()) as Arc<dyn ac_sandbox::Sandbox>)
    } else {
        None
    };
    let fx = fixture_with(Opts {
        real_llm: true,
        sandbox,
        summary_model: Some("anthropic:claude-haiku-5-5".into()),
        ..Default::default()
    })
    .await;
    let (dba, _) = fx
        .agent_with(
            "dba",
            "anthropic:claude-sonnet-5-5",
            "資料庫管理員：資料庫效能調校、查詢優化、索引設計、連線池設定",
            vec!["PostgreSQL", "慢查詢", "索引", "連線池"],
        )
        .await;
    let (sre, _) = fx
        .agent_with(
            "sre",
            "openai:gpt-6-luna",
            "維運工程師：部署、Kubernetes、監控告警、事故處理",
            vec!["Kubernetes", "部署", "監控"],
        )
        .await;
    let (designer, _) = fx
        .agent_with(
            "designer",
            "gemini:gemini-4-hydrogen",
            "設計師：UI/UX 設計、設計稿、配色與使用者體驗",
            vec!["Figma", "配色", "UX"],
        )
        .await;
    let ch = fx.channel(&[dba.member.id, sre.member.id, designer.member.id]).await;

    // 1. Each provider answers when tagged (tool calling via post_message).
    for (agent, q) in [
        (&dba, "@dba 用一句話說明：為什麼查詢很慢時要先看 EXPLAIN？"),
        (&sre, "@sre 用一句話說明：Kubernetes 的 readiness probe 是做什麼的？"),
        (&designer, "@designer 用一句話建議：深色模式的文字對比度至少要多少？"),
    ] {
        let m = fx.svc().post_message(&fx.human, None, ch.id, q, None).await.unwrap().message;
        let reply = agent_reply(&fx, ch.id, agent.member.id, m.seq, 120).await;
        println!("[{}] {}", agent.member.handle, reply);
        assert!(has_cjk(&reply), "reply in Chinese: {reply}");
        wait_for("wake finished", Duration::from_secs(120), || async {
            fx.store().wakes_for_trigger(m.id).await.unwrap().iter().all(|w| w.status.is_terminal())
        })
        .await;
        let w = fx.store().wakes_for_trigger(m.id).await.unwrap();
        assert!(w.iter().all(|w| w.status == WakeStatus::Done && w.tokens_used > 0 && w.posted_count >= 1), "{w:?}");
    }
    wait_for("settled", Duration::from_secs(60), || settled(&fx, ch.id)).await;

    // 2. The real Decisions API routes an untagged message.
    let key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY");
    fx.svc()
        .create_decider_config(
            &fx.human,
            DeciderConfigInput {
                name: Some("Gemini Decisions API".into()),
                protocol: Some(DeciderProtocol::DecisionsApi),
                endpoint_url: Some("https://generativelanguage.googleapis.com/v1beta/decisions".into()),
                api_key: Some(key),
                model: Some("gemini-decision-1".into()),
                timeout_ms: Some(5000),
                t_wake: Some(0.60),
                t_discuss: Some(0.75),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // A fresh channel: in `ch` the human just got answers, so rule 4 (sticky) would take precedence.
    let ch = fx.channel(&[dba.member.id, sre.member.id, designer.member.id]).await;
    let m = fx
        .svc()
        .post_message(&fx.human, None, ch.id, "資料庫連線池一直滿，max_connections 要怎麼調比較好？", None)
        .await
        .unwrap()
        .message;
    let reply = agent_reply(&fx, ch.id, dba.member.id, m.seq, 120).await;
    println!("[decider → dba] {reply}");
    let w = fx.store().wakes_for_trigger(m.id).await.unwrap();
    assert!(w.iter().any(|w| w.agent_id == dba.member.id && w.reason == WakeReason::Decider), "{w:?}");
    assert!(!w.iter().any(|w| w.agent_id == designer.member.id), "designer should not be woken");
    let d = fx.store().decisions_for_message(m.id).await.unwrap();
    println!("decision: scores={:?} latency={:?}ms", d[0].scores, d[0].latency_ms);
    wait_for("settled", Duration::from_secs(120), || settled(&fx, ch.id)).await;

    // 3. A question that asks two agents to debate → discussion (or parallel if the Decider says so).
    let ch = fx.channel(&[dba.member.id, sre.member.id, designer.member.id]).await;
    let m = fx
        .svc()
        .post_message(
            &fx.human,
            None,
            ch.id,
            "我們該不該把資料庫從 VM 搬到 Kubernetes 上跑？請資料庫和維運兩位互相討論利弊，最後給一個共識結論。",
            None,
        )
        .await
        .unwrap()
        .message;
    wait_for("decision", Duration::from_secs(30), || async {
        !fx.store().decisions_for_message(m.id).await.unwrap().is_empty()
    })
    .await;
    let d = fx.store().decisions_for_message(m.id).await.unwrap().remove(0);
    println!(
        "discussion decision: mode={:?} interaction={:?}({:?}) scores={:?}",
        d.mode, d.interaction, d.interaction_confidence, d.scores
    );
    if d.mode.as_deref() == Some(WakeMode::Discussion.as_str()) {
        wait_for("discussion done", Duration::from_secs(600), || async {
            fx.store().active_discussion(ch.id, m.id).await.unwrap().is_none() && settled(&fx, ch.id).await
        })
        .await;
        let wakes = fx.store().wakes_for_trigger(m.id).await.unwrap();
        let disc = fx.store().get_discussion(wakes[0].discussion_id.unwrap()).await.unwrap().unwrap();
        println!(
            "discussion: status={} reason={:?} rounds={} wakes={}",
            disc.status,
            disc.end_reason,
            disc.round,
            wakes.len()
        );
        assert_eq!(disc.status, DiscussionStatus::Done);
        let thread = fx.store().list_messages(ch.id, MessageScope::Thread(m.id), None, 100).await.unwrap();
        for t in &thread {
            println!("  [{}] {}", t.author_handle, t.body.chars().take(160).collect::<String>());
        }
        assert!(thread.iter().filter(|t| t.author_kind == MemberKind::Agent).count() >= 2);
        let main = fx.store().list_messages(ch.id, MessageScope::Main, None, 10).await.unwrap();
        assert!(
            main.iter().any(|x| x.seq > m.seq && x.author_kind == MemberKind::Agent),
            "conclusion mirrored to the main conversation"
        );
    } else {
        wait_for("settled", Duration::from_secs(240), || settled(&fx, ch.id)).await;
        println!("decider chose {:?}; wakes: {}", d.mode, fx.store().wakes_for_trigger(m.id).await.unwrap().len());
        assert!(!d.woken.is_empty());
    }

    // 4. Sandbox with a real model: compute, write to long-term memory, report back.
    if sandbox_on {
        let m = fx
            .svc()
            .post_message(
                &fx.human,
                None,
                ch.id,
                "@dba 請在你的沙箱用 python 算出 2 的 100 次方，把結果寫進 /workspace/memory/result.txt，然後在這裡回報那個數字。",
                None,
            )
            .await
            .unwrap()
            .message;
        let reply = agent_reply(&fx, ch.id, dba.member.id, m.seq, 240).await;
        println!("[sandbox] {reply}");
        assert!(reply.replace([',', ' '], "").contains("1267650600228229401496703205376"), "{reply}");
        let file = fx
            .svc()
            .call_tool(
                &fx.store().get_member(dba.member.id).await.unwrap().unwrap(),
                None,
                "read_file",
                serde_json::json!({"path": "memory/result.txt"}),
            )
            .await
            .unwrap();
        assert!(file["content"].as_str().unwrap().contains("1267650600228229401496703205376"));
        for a in [&dba, &sre, &designer] {
            fx.svc().delete_agent(&fx.human, a.member.id, true).await.unwrap();
        }
    }
}
