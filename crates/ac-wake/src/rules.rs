//! Per-message wake decision (spec 6 flow, 6.1 rule layer, 6.6 limits).

use ac_core::bus::RealtimeEvent;
use ac_core::model::{
    Conversation, ConversationKind, ConversationMemberView, DecisionRecord, Member, MemberKind, Message, WakeMode,
    WakeReason,
};
use ac_core::{AgentId, AppError, AppResult, new_id};
use ac_store::{MessageScope, NewWake};
use chrono::Utc;
use serde_json::json;

use crate::{WakeEngine, start_of_day};

pub(crate) const LIMIT_NOTICE: &str = "已達自動對話上限，請人類繼續。";

/// What the rule layer produced.
pub(crate) enum RuleOutcome {
    Wake(Vec<AgentId>, WakeReason),
    /// Several agents spoke in the thread: the Decider picks among them.
    DecideAmong(Vec<AgentId>),
    Nothing,
}

impl WakeEngine {
    pub(crate) async fn process_message(&self, msg: Message) -> AppResult<()> {
        let store = &self.inner.store;
        let limits = &self.inner.limits;
        if msg.deleted_at.is_some() {
            return Ok(());
        }
        let author = store.get_member(msg.author_id).await?.ok_or_else(|| AppError::not_found("author"))?;
        if author.kind == MemberKind::System {
            return Ok(());
        }
        let mut conv =
            store.get_conversation(msg.conversation_id).await?.ok_or_else(|| AppError::not_found("conversation"))?;

        // Frozen conversations: agent messages wake nobody; a human message unfreezes.
        if conv.frozen_until_human {
            if author.kind == MemberKind::Human {
                conv = store.set_frozen(conv.id, false).await?;
                self.inner.bus.publish(RealtimeEvent::ConversationUpdated { conversation: conv.clone() });
            } else {
                tracing::debug!(message_id = %msg.id, "conversation frozen; agent message wakes nobody");
                return Ok(());
            }
        }

        // Chain budget.
        let (chain_wakes, chain_tokens) = store.chain_stats(msg.chain_root_id).await?;
        if chain_wakes >= limits.chain_max_wakes {
            self.limit_notice(&msg, "chain_wakes", "這條對話鏈的自動喚醒次數已達上限").await?;
            return Ok(());
        }
        if let Some(cap) = limits.chain_max_tokens
            && chain_tokens >= cap
        {
            for (w, _) in store.cancel_pending_chain_wakes(msg.chain_root_id).await? {
                self.publish_wake(&w).await;
            }
            self.limit_notice(&msg, "chain_tokens", "這條對話鏈的 token 用量已達上限").await?;
            return Ok(());
        }

        // Active discussion in this thread → discussion scheduling (6.5).
        if let Some(root) = msg.thread_root_id
            && let Some(d) = store.active_discussion(conv.id, root).await?
        {
            return self.discussion_message(d, &msg, &author).await;
        }

        // Depth.
        if msg.depth >= limits.max_depth {
            self.limit_notice(&msg, "depth", &format!("對話深度已達 {}", limits.max_depth)).await?;
            return Ok(());
        }

        let members = store.conversation_members(conv.id).await?;
        match self.rule_layer(&conv, &members, &msg, &author).await? {
            RuleOutcome::Wake(targets, reason) => {
                let mode = if targets.len() > 1 { WakeMode::Parallel } else { WakeMode::Single };
                let woken = self.schedule(&msg, &conv, targets, reason, mode, chain_wakes).await?;
                self.record_rule_decision(&msg, reason, &woken, mode).await?;
            }
            RuleOutcome::DecideAmong(candidates) => {
                self.spawn_decider(msg, conv, members, Some(candidates));
            }
            RuleOutcome::Nothing => {
                // Agent messages are never sent to the Decider (spec 6, open question 4).
                if author.kind == MemberKind::Human {
                    self.spawn_decider(msg, conv, members, None);
                }
            }
        }
        Ok(())
    }

    async fn rule_layer(
        &self,
        conv: &Conversation,
        members: &[ConversationMemberView],
        msg: &Message,
        author: &Member,
    ) -> AppResult<RuleOutcome> {
        let store = &self.inner.store;
        let is_agent_member = |id: AgentId| members.iter().any(|m| m.member_id == id && m.kind == MemberKind::Agent);

        // 1. explicit @mentions (already restricted to conversation members at post time)
        let mut targets: Vec<AgentId> = Vec::new();
        for id in &msg.mentions {
            if *id != msg.author_id && is_agent_member(*id) && !targets.contains(id) {
                targets.push(*id);
            }
        }
        if !targets.is_empty() {
            return Ok(RuleOutcome::Wake(targets, WakeReason::Mention));
        }

        // 2. DM: every other agent in the DM
        if conv.kind == ConversationKind::Dm {
            let t: Vec<AgentId> = members
                .iter()
                .filter(|m| m.kind == MemberKind::Agent && m.member_id != msg.author_id)
                .map(|m| m.member_id)
                .collect();
            if !t.is_empty() {
                return Ok(RuleOutcome::Wake(t, WakeReason::Dm));
            }
        }

        // 3. thread reply: agents that spoke in the thread
        if let Some(root) = msg.thread_root_id {
            let thread = store.list_messages(conv.id, MessageScope::Thread(root), Some(msg.seq), 100).await?;
            let mut speakers: Vec<AgentId> = Vec::new();
            for m in thread.iter().rev() {
                if m.author_kind == MemberKind::Agent
                    && m.author_id != msg.author_id
                    && is_agent_member(m.author_id)
                    && !speakers.contains(&m.author_id)
                {
                    speakers.push(m.author_id);
                }
            }
            // Agents tagged in the thread root are part of the thread too (they are usually the ones
            // who were asked — even if they answered in the main timeline).
            if speakers.is_empty() {
                if let Some(r) = thread.iter().find(|m| m.id == root) {
                    if let Some(rm) = store.get_message(r.id).await? {
                        for id in rm.mentions {
                            if id != msg.author_id && is_agent_member(id) && !speakers.contains(&id) {
                                speakers.push(id);
                            }
                        }
                    }
                }
            }
            match speakers.len() {
                0 => {}
                1 => return Ok(RuleOutcome::Wake(speakers, WakeReason::ThreadReply)),
                _ if author.kind == MemberKind::Human => return Ok(RuleOutcome::DecideAmong(speakers)),
                // Agents never go to the Decider: the most recent speaker continues.
                _ => return Ok(RuleOutcome::Wake(vec![speakers[0]], WakeReason::ThreadReply)),
            }
        }

        // 4. sticky: a human replying right after an agent answered that human
        if author.kind == MemberKind::Human {
            let scope = MessageScope::from_thread(msg.thread_root_id);
            if let Some(prev) = store.previous_other_message(conv.id, scope, msg.seq, msg.author_id).await? {
                let within =
                    (msg.created_at - prev.created_at).num_seconds() <= self.inner.limits.sticky_window_seconds;
                if within && is_agent_member(prev.author_id) && prev.wake_id.is_some() {
                    let root_author = store.get_message(prev.chain_root_id).await?.map(|m| m.author_id);
                    // Platform agents always answer when called, so an untagged follow-up in the main
                    // timeline does not wake them (a reply in their thread still does).
                    let mention_only = store.get_agent(prev.author_id).await?.is_some_and(|a| a.mention_only);
                    if root_author == Some(msg.author_id) && !mention_only {
                        return Ok(RuleOutcome::Wake(vec![prev.author_id], WakeReason::Sticky));
                    }
                }
            }
        }
        Ok(RuleOutcome::Nothing)
    }

    /// Filters by daily budgets and the chain cap, then creates (or merges) the wakes.
    /// Returns the agents actually scheduled.
    pub(crate) async fn schedule(
        &self,
        msg: &Message,
        conv: &Conversation,
        targets: Vec<AgentId>,
        reason: WakeReason,
        mode: WakeMode,
        chain_wakes: i64,
    ) -> AppResult<Vec<AgentId>> {
        let store = &self.inner.store;
        let limits = &self.inner.limits;
        let allowed = self.within_daily_budget(conv, msg, targets).await?;
        if allowed.is_empty() {
            return Ok(vec![]);
        }
        let mut allowed = allowed;
        let remaining = (limits.chain_max_wakes - chain_wakes).max(0) as usize;
        if allowed.len() > remaining {
            allowed.truncate(remaining);
            self.limit_notice(msg, "chain_wakes", "這條對話鏈的自動喚醒次數已達上限").await?;
        }
        let mode = if allowed.len() > 1 { mode } else { WakeMode::Single };
        for agent_id in &allowed {
            let (wake, merged) = store
                .create_or_merge_wake(&NewWake {
                    agent_id: *agent_id,
                    conversation_id: conv.id,
                    thread_root_id: msg.thread_root_id,
                    trigger_message_id: msg.id,
                    reason,
                    mode,
                    discussion_id: None,
                    depth: msg.depth,
                    chain_root_id: msg.chain_root_id,
                })
                .await?;
            tracing::info!(wake_id = %wake.id, agent_id = %agent_id, %reason, %mode, merged,
                chain_root_id = %msg.chain_root_id, depth = msg.depth, "wake scheduled");
            if !merged {
                self.publish_wake(&wake).await;
            }
            self.ensure_worker(*agent_id).await;
        }
        Ok(allowed)
    }

    pub(crate) async fn within_daily_budget(
        &self,
        conv: &Conversation,
        msg: &Message,
        targets: Vec<AgentId>,
    ) -> AppResult<Vec<AgentId>> {
        let store = &self.inner.store;
        let since = start_of_day(Utc::now());
        let mut allowed = Vec::new();
        for agent_id in targets {
            let Some(agent) = store.get_agent(agent_id).await? else { continue };
            if agent.member.deactivated_at.is_some() {
                continue;
            }
            if let Some(budget) = agent.daily_token_budget {
                let used = store.agent_tokens_since(agent_id, since).await?;
                if used >= budget {
                    let first = self.inner.budget_notices.lock().unwrap().insert((agent_id, since.date_naive()));
                    if first {
                        self.post_system(
                            conv.id,
                            msg.thread_root_id,
                            format!("@{} 今日的 token 預算已用完，今天不會再被喚醒。", agent.member.handle),
                        )
                        .await?;
                    }
                    continue;
                }
            }
            allowed.push(agent_id);
        }
        Ok(allowed)
    }

    async fn record_rule_decision(
        &self,
        msg: &Message,
        reason: WakeReason,
        woken: &[AgentId],
        mode: WakeMode,
    ) -> AppResult<()> {
        self.inner
            .store
            .insert_decision(&DecisionRecord {
                id: new_id(),
                trigger_message_id: msg.id,
                decider_config_id: None,
                source: "rule".into(),
                reason: Some(reason.as_str().into()),
                attempts: json!([]),
                request: None,
                response: None,
                scores: None,
                interaction: None,
                interaction_confidence: None,
                woken: woken.to_vec(),
                mode: Some(mode.as_str().into()),
                latency_ms: Some(0),
                created_at: Utc::now(),
            })
            .await
    }

    /// Posts the "automatic conversation limit" notice once per chain.
    pub(crate) async fn limit_notice(&self, msg: &Message, kind: &str, detail: &str) -> AppResult<()> {
        tracing::info!(chain_root_id = %msg.chain_root_id, kind, "chain limit reached");
        if self.inner.store.try_insert_chain_notice(msg.chain_root_id, kind).await? {
            self.post_system(msg.conversation_id, msg.thread_root_id, format!("{detail}。{LIMIT_NOTICE}")).await?;
        }
        Ok(())
    }
}
