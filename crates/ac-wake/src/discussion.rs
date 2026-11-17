//! Discussion mode (spec 6.5): agents take turns in a thread under the trigger message,
//! for at most `max_rounds` rounds; then a summarizer posts the conclusion in the thread
//! and mirrors it to the main conversation.

use ac_core::model::{
    Agent, Conversation, Discussion, DiscussionStatus, Member, MemberKind, Message, Wake, WakeMode, WakeReason,
};
use ac_core::protocol::PayloadDiscussion;
use ac_core::{AgentId, AppResult};
use ac_store::NewWake;

use crate::WakeEngine;
use crate::rules::LIMIT_NOTICE;

fn end_reason_text(r: &str) -> &'static str {
    match r {
        "all_skipped" => "這一輪所有參與者都沒有新觀點",
        "max_rounds" => "已達回合上限",
        "budget" => "自動對話預算已用完",
        "stopped" => "人類喊停",
        _ => "討論結束",
    }
}

impl WakeEngine {
    /// Starts a discussion for `msg` among `participants` (ordered by score, highest first).
    pub(crate) async fn start_discussion(
        &self,
        msg: &Message,
        conv: &Conversation,
        participants: Vec<AgentId>,
        chain_wakes: i64,
    ) -> AppResult<Vec<AgentId>> {
        let store = &self.inner.store;
        let limits = &self.inner.limits;
        let mut allowed = self.within_daily_budget(conv, msg, participants).await?;
        allowed.truncate(limits.discussion_max_participants);
        if allowed.len() < 2 {
            return self.schedule(msg, conv, allowed, WakeReason::Decider, WakeMode::Single, chain_wakes).await;
        }
        let _g = self.inner.discussions.lock().await;
        let root = msg.thread_root_id.unwrap_or(msg.id);
        if store.active_discussion(conv.id, root).await?.is_some() {
            return Ok(vec![]);
        }
        let d = store
            .create_discussion(
                conv.id,
                root,
                &allowed,
                limits.discussion_max_rounds,
                allowed[0],
                msg.id,
                msg.chain_root_id,
                msg.depth,
            )
            .await?;
        let handles = self.handles(&allowed).await?;
        self.post_system(
            conv.id,
            Some(root),
            format!(
                "開始討論：{}（最多 {} 輪，依序發言）。在這個 thread 發言可以補充或引導討論。",
                handles.iter().map(|h| format!("@{h}")).collect::<Vec<_>>().join("、"),
                d.max_rounds
            ),
        )
        .await?;
        self.schedule_turn(&d, d.participants[0]).await?;
        Ok(allowed)
    }

    async fn handles(&self, ids: &[AgentId]) -> AppResult<Vec<String>> {
        let members = self.inner.store.get_members(ids).await?;
        Ok(ids
            .iter()
            .map(|id| members.iter().find(|m| m.id == *id).map(|m| m.handle.clone()).unwrap_or_default())
            .collect())
    }

    async fn schedule_turn(&self, d: &Discussion, agent_id: AgentId) -> AppResult<()> {
        let (wake, _) = self
            .inner
            .store
            .create_or_merge_wake(&NewWake {
                agent_id,
                conversation_id: d.conversation_id,
                thread_root_id: Some(d.thread_root_id),
                trigger_message_id: d.trigger_message_id.unwrap_or(d.thread_root_id),
                reason: WakeReason::DiscussionTurn,
                mode: WakeMode::Discussion,
                discussion_id: Some(d.id),
                depth: d.depth,
                chain_root_id: d.chain_root_id.unwrap_or(d.thread_root_id),
            })
            .await?;
        tracing::info!(discussion_id = %d.id, round = d.round, turn = d.turn_index, agent = %agent_id, "discussion turn scheduled");
        self.publish_wake(&wake).await;
        self.kick_worker(agent_id);
        Ok(())
    }

    /// A message in a thread with an active discussion (participants' messages skip the rule layer).
    pub(crate) async fn discussion_message(&self, d: Discussion, msg: &Message, author: &Member) -> AppResult<()> {
        let _g = self.inner.discussions.lock().await;
        let store = &self.inner.store;
        let Some(mut d) = store.get_discussion(d.id).await? else { return Ok(()) };
        if d.status != DiscussionStatus::Running {
            return Ok(());
        }
        let mut changed = false;
        if author.kind == MemberKind::Human {
            d.human_interjected = true;
            changed = true;
        }
        if author.kind == MemberKind::Human || d.participants.contains(&author.id) {
            // Tagging a non-participant agent pulls it into the discussion (end of this round).
            let members = store.conversation_members(d.conversation_id).await?;
            for id in &msg.mentions {
                let is_agent = members.iter().any(|m| m.member_id == *id && m.kind == MemberKind::Agent);
                if is_agent
                    && !d.participants.contains(id)
                    && d.participants.len() < self.inner.limits.discussion_max_participants
                {
                    d.participants.push(*id);
                    changed = true;
                    tracing::info!(discussion_id = %d.id, agent = %id, "agent joined discussion");
                }
            }
        }
        if changed {
            store.save_discussion(&d).await?;
        }
        Ok(())
    }

    /// Advances the discussion after one of its wakes reached a terminal state.
    pub(crate) async fn on_discussion_wake_finished(&self, wake_id: uuid::Uuid) -> AppResult<()> {
        let _g = self.inner.discussions.lock().await;
        let store = &self.inner.store;
        let Some(wake) = store.get_wake(wake_id).await? else { return Ok(()) };
        let Some(did) = wake.discussion_id else { return Ok(()) };
        let Some(mut d) = store.get_discussion(did).await? else { return Ok(()) };
        match d.status {
            DiscussionStatus::Summarizing => {
                if Some(wake.agent_id) == d.summarizer_id {
                    let reason = d.end_reason.clone().unwrap_or_default();
                    store.set_discussion_status(d.id, DiscussionStatus::Done, &reason).await?;
                    self.post_system(
                        d.conversation_id,
                        Some(d.thread_root_id),
                        format!(
                            "討論已結束（{}，共 {} 輪）。",
                            end_reason_text(&reason),
                            (d.round + 1).min(d.max_rounds)
                        ),
                    )
                    .await?;
                }
                return Ok(());
            }
            DiscussionStatus::Running => {}
            _ => return Ok(()),
        }
        if wake.posted_count > 0 {
            d.spoke_this_round = true;
        }

        // Discussions are not bound by MAX_DEPTH but still by the chain budget.
        let chain = d.chain_root_id.unwrap_or(d.thread_root_id);
        let (count, tokens) = store.chain_stats(chain).await?;
        let over_tokens = self.inner.limits.chain_max_tokens.map(|cap| tokens >= cap).unwrap_or(false);
        if count >= self.inner.limits.chain_max_wakes || over_tokens {
            d.status = DiscussionStatus::Done;
            d.end_reason = Some("budget".into());
            store.save_discussion(&d).await?;
            if store.try_insert_chain_notice(chain, "discussion_budget").await? {
                self.post_system(
                    d.conversation_id,
                    Some(d.thread_root_id),
                    format!("討論因自動對話預算用完而結束。{LIMIT_NOTICE}"),
                )
                .await?;
            }
            return Ok(());
        }

        d.turn_index += 1;
        if d.turn_index as usize >= d.participants.len() {
            if d.human_interjected {
                // A human spoke: run another round without counting it.
                d.human_interjected = false;
            } else if !d.spoke_this_round {
                return self.conclude(d, "all_skipped").await;
            } else {
                d.round += 1;
                if d.round >= d.max_rounds {
                    return self.conclude(d, "max_rounds").await;
                }
            }
            d.turn_index = 0;
            d.spoke_this_round = false;
        }
        store.save_discussion(&d).await?;
        let next = d.participants[d.turn_index as usize];
        self.schedule_turn(&d, next).await
    }

    async fn conclude(&self, mut d: Discussion, reason: &str) -> AppResult<()> {
        d.status = DiscussionStatus::Summarizing;
        d.end_reason = Some(reason.into());
        self.inner.store.save_discussion(&d).await?;
        let summarizer = d.summarizer_id.unwrap_or(d.participants[0]);
        self.schedule_turn(&d, summarizer).await
    }

    /// Ends every active discussion of a conversation (human stop).
    pub(crate) async fn cancel_discussions(&self, conversation_id: uuid::Uuid) -> AppResult<()> {
        let _g = self.inner.discussions.lock().await;
        for d in self.inner.store.active_discussions_in_conversation(conversation_id).await? {
            self.inner.store.set_discussion_status(d.id, DiscussionStatus::Cancelled, "stopped").await?;
        }
        Ok(())
    }

    /// The `discussion` part of a wake payload.
    pub(crate) async fn discussion_payload(&self, wake: &Wake, agent: &Agent) -> AppResult<Option<PayloadDiscussion>> {
        let Some(did) = wake.discussion_id else { return Ok(None) };
        let Some(d) = self.inner.store.get_discussion(did).await? else { return Ok(None) };
        let handles = self.handles(&d.participants).await?;
        let list = handles.iter().map(|h| format!("@{h}")).collect::<Vec<_>>().join("、");
        let round = (d.round + 1).min(d.max_rounds);
        let conclusion = d.status == DiscussionStatus::Summarizing && Some(agent.member.id) == d.summarizer_id;
        let instruction = if conclusion {
            format!(
                "討論已結束（{}）。你負責整理結論：先用 post_message 在 thread 中（thread_root_id={}）貼出結論，\
                 包含共識、分歧與後續行動；再用 post_message（不帶 thread_root_id）把同一份結論同步貼回主對話。",
                end_reason_text(d.end_reason.as_deref().unwrap_or("")),
                d.thread_root_id
            )
        } else {
            format!(
                "這是多 Agent 討論（第 {round}/{} 輪），參與者依序為 {list}，現在輪到你。請閱讀 thread 中其他人的發言，\
                 提出新的觀點、補充或反駁，並用 post_message 在 thread 中發言（thread_root_id={}）。\
                 若沒有新的內容可以補充，就不要呼叫 post_message，視為這一輪跳過。",
                d.max_rounds, d.thread_root_id
            )
        };
        Ok(Some(PayloadDiscussion {
            id: d.id,
            phase: if conclusion { "conclusion".into() } else { "turn".into() },
            round,
            max_rounds: d.max_rounds,
            participants: handles,
            instruction,
        }))
    }
}
