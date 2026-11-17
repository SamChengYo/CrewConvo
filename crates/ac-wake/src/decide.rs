//! Decider path (spec 6.2): runs off the write path in its own task; failures never block
//! messages and are recorded in `decisions`.

use std::sync::atomic::Ordering;

use ac_core::model::{
    AgentProfile, AgentStatus, Conversation, ConversationKind, ConversationMemberView, DecisionRecord, MemberKind,
    Message, WakeMode, WakeReason,
};
use ac_core::{AgentId, AppResult, new_id};
use ac_decider::{DecideConversation, DecideInput, DecideMessage, RECENT_COUNT, resolve};
use ac_store::MessageScope;
use chrono::Utc;
use serde_json::{Map, Value, json};

use crate::WakeEngine;

impl WakeEngine {
    pub(crate) fn spawn_decider(
        &self,
        msg: Message,
        conv: Conversation,
        members: Vec<ConversationMemberView>,
        restrict: Option<Vec<AgentId>>,
    ) {
        self.inner.background.fetch_add(1, Ordering::SeqCst);
        let e = self.clone();
        tokio::spawn(async move {
            let id = msg.id;
            if let Err(err) = e.run_decider(msg, conv, members, restrict).await {
                tracing::error!(message_id = %id, error = %err, "decider path failed");
            }
            e.inner.background.fetch_sub(1, Ordering::SeqCst);
        });
    }

    /// Builds the Decider input for a message (also used by the "test" button and evaluation).
    pub async fn decide_input(
        &self,
        msg: &Message,
        conv: &Conversation,
        candidates: Vec<AgentProfile>,
    ) -> AppResult<DecideInput> {
        let store = &self.inner.store;
        let author = store.get_member(msg.author_id).await?;
        let scope = MessageScope::from_thread(msg.thread_root_id);
        let recent = store.list_messages(conv.id, scope, Some(msg.seq), RECENT_COUNT as i64).await?;
        Ok(DecideInput {
            message: DecideMessage {
                author_handle: author.map(|a| a.handle).unwrap_or_default(),
                author_kind: MemberKind::Human,
                body: msg.body.clone(),
            },
            recent: recent
                .iter()
                .filter(|m| m.author_kind != MemberKind::System)
                .map(|m| DecideMessage {
                    author_handle: m.author_handle.clone(),
                    author_kind: m.author_kind,
                    body: m.body.clone(),
                })
                .collect(),
            conversation: DecideConversation {
                name: conv.name.clone(),
                topic: conv.topic.clone(),
                is_dm: conv.kind == ConversationKind::Dm,
            },
            candidates,
        })
    }

    async fn run_decider(
        &self,
        msg: Message,
        conv: Conversation,
        members: Vec<ConversationMemberView>,
        restrict: Option<Vec<AgentId>>,
    ) -> AppResult<()> {
        let store = &self.inner.store;
        let ids: Vec<AgentId> = members
            .iter()
            .filter(|m| m.kind == MemberKind::Agent && m.member_id != msg.author_id)
            .filter(|m| restrict.as_ref().map(|r| r.contains(&m.member_id)).unwrap_or(true))
            .map(|m| m.member_id)
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let mut candidates: Vec<AgentProfile> = store
            .get_agents(&ids)
            .await?
            .iter()
            .filter(|a| {
                // Platform agents always answer when called, so the model never picks them.
                !a.mention_only
                    && a.member.deactivated_at.is_none()
                    && matches!(a.status, AgentStatus::Online | AgentStatus::Busy)
            })
            .map(AgentProfile::from)
            .collect();
        // Keep the restrict order (most recent thread speaker first) for fallbacks.
        if let Some(r) = &restrict {
            candidates.sort_by_key(|c| r.iter().position(|x| *x == c.id).unwrap_or(usize::MAX));
        }
        if candidates.is_empty() {
            return Ok(());
        }
        let fallback_reason = WakeReason::ThreadReply;
        let configs = store.list_decider_configs(conv.workspace_id).await?;
        if !configs.iter().any(|c| c.config.enabled) {
            if let Some(r) = &restrict {
                // No Decider available: the most recent thread speaker continues.
                let (chain_wakes, _) = store.chain_stats(msg.chain_root_id).await?;
                let woken =
                    self.schedule(&msg, &conv, vec![r[0]], fallback_reason, WakeMode::Single, chain_wakes).await?;
                self.record(
                    &msg,
                    None,
                    "rule",
                    Some("thread_reply_fallback"),
                    json!([]),
                    None,
                    None,
                    None,
                    None,
                    None,
                    &woken,
                    Some(WakeMode::Single),
                    0,
                )
                .await?;
            }
            return Ok(());
        }

        let input = self.decide_input(&msg, &conv, candidates.clone()).await?;
        let started = std::time::Instant::now();
        let result = self.inner.decider.decide(&configs, &input).await;
        let attempts = serde_json::to_value(&result.attempts).unwrap_or(json!([]));
        let total_ms = started.elapsed().as_millis() as i32;

        let Some((cfg, call)) = result.used else {
            tracing::warn!(message_id = %msg.id, "all deciders failed; waking nobody");
            let mut woken = vec![];
            if let Some(r) = &restrict {
                let (chain_wakes, _) = store.chain_stats(msg.chain_root_id).await?;
                woken = self.schedule(&msg, &conv, vec![r[0]], fallback_reason, WakeMode::Single, chain_wakes).await?;
            }
            self.record(
                &msg,
                None,
                "decider",
                Some("all_failed"),
                attempts,
                None,
                None,
                None,
                None,
                None,
                &woken,
                None,
                total_ms,
            )
            .await?;
            return Ok(());
        };

        let (woken_scores, mode) = resolve(&call.decision, &cfg);
        let scores: Map<String, Value> = call
            .decision
            .scores
            .iter()
            .map(|(id, s)| {
                let h =
                    candidates.iter().find(|c| c.id == *id).map(|c| c.handle.clone()).unwrap_or_else(|| id.to_string());
                (h, json!(s))
            })
            .collect();
        let targets: Vec<AgentId> = woken_scores.iter().map(|(id, _)| *id).collect();
        let reason = if restrict.is_some() { WakeReason::ThreadReply } else { WakeReason::Decider };
        let mut woken = Vec::new();
        if let Some(mode) = mode {
            let (chain_wakes, _) = store.chain_stats(msg.chain_root_id).await?;
            woken = if mode == WakeMode::Discussion {
                self.start_discussion(&msg, &conv, targets, chain_wakes).await?
            } else {
                self.schedule(&msg, &conv, targets, reason, mode, chain_wakes).await?
            };
        }
        self.record(
            &msg,
            Some(cfg.id),
            "decider",
            Some(reason.as_str()),
            attempts,
            Some(call.request),
            Some(call.response),
            Some(Value::Object(scores)),
            Some(call.decision.interaction.as_str().to_string()),
            Some(call.decision.interaction_confidence),
            &woken,
            mode,
            call.latency_ms as i32,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn record(
        &self,
        msg: &Message,
        config_id: Option<uuid::Uuid>,
        source: &str,
        reason: Option<&str>,
        attempts: Value,
        request: Option<Value>,
        response: Option<Value>,
        scores: Option<Value>,
        interaction: Option<String>,
        interaction_confidence: Option<f32>,
        woken: &[AgentId],
        mode: Option<WakeMode>,
        latency_ms: i32,
    ) -> AppResult<()> {
        self.inner
            .store
            .insert_decision(&DecisionRecord {
                id: new_id(),
                trigger_message_id: msg.id,
                decider_config_id: config_id,
                source: source.into(),
                reason: reason.map(String::from),
                attempts,
                request,
                response,
                scores,
                interaction,
                interaction_confidence,
                woken: woken.to_vec(),
                mode: mode.map(|m| m.as_str().to_string()),
                latency_ms: Some(latency_ms),
                created_at: Utc::now(),
            })
            .await
    }
}
