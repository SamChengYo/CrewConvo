//! Wake payload assembly (spec 7): cursor, recent/unread windows, token budgets, rolling summaries.

pub mod summary;
pub use summary::{Summarizer, SummaryConfig};

use ac_core::model::{Agent, MessageView, Wake, WakeStatus};
use ac_core::protocol::{PayloadConversation, PayloadLimits, PayloadMember, PayloadMessage, WakePayload};
use ac_core::{AppError, AppResult, estimate_tokens, limits::Limits};
use ac_store::{MessageScope, Store};

#[derive(Debug, Clone)]
pub struct ContextBudget {
    /// recent + unread token budget.
    pub messages_tokens: usize,
    /// Max messages fetched as unread before trimming.
    pub unread_fetch: i64,
    /// Max messages of already-read context.
    pub recent_fetch: i64,
    pub private_logs: i64,
    pub summary_tokens: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self { messages_tokens: 8000, unread_fetch: 200, recent_fetch: 30, private_logs: 3, summary_tokens: 1000 }
    }
}

/// Token estimate of the messages part of a payload (used by tests and metrics).
pub fn payload_message_tokens(p: &WakePayload) -> usize {
    p.recent.iter().chain(p.unread.iter()).map(|m| estimate_tokens(&m.body) + 24).sum()
}

#[derive(Clone)]
pub struct ContextAssembler {
    store: Store,
    limits: Limits,
    budget: ContextBudget,
}

/// A single message never takes more than this in a payload; longer bodies are clipped
/// (the agent can read the full text with `read_history`).
pub const MAX_MESSAGE_TOKENS: usize = 2000;

fn message_tokens(m: &MessageView) -> usize {
    // body + small per-message overhead for metadata
    estimate_tokens(&m.body).min(MAX_MESSAGE_TOKENS + 20) + 24
}

fn to_payload(m: &MessageView) -> PayloadMessage {
    let mut p = PayloadMessage::from(m);
    if estimate_tokens(&p.body) > MAX_MESSAGE_TOKENS {
        p.body = format!(
            "{}（訊息過長已截斷，完整內容請用 read_history 讀取）",
            summary::clip_tokens(&p.body, MAX_MESSAGE_TOKENS)
        );
    }
    p
}

impl ContextAssembler {
    pub fn new(store: Store, limits: Limits, budget: ContextBudget) -> Self {
        Self { store, limits, budget }
    }

    pub async fn assemble(&self, wake: &Wake, agent: &Agent) -> AppResult<WakePayload> {
        let conv = self
            .store
            .get_conversation(wake.conversation_id)
            .await?
            .ok_or_else(|| AppError::not_found("conversation"))?;
        let members = self.store.conversation_members(conv.id).await?;
        let scope = MessageScope::from_thread(wake.thread_root_id);
        let thread_root = match wake.thread_root_id {
            Some(r) => self.store.get_message_view(r).await?.map(|m| to_payload(&m)),
            None => None,
        };

        let cursor = self.store.get_cursor(conv.id, agent.member.id).await?;
        let (unread_all, unread_total) =
            self.store.messages_after(conv.id, scope, cursor, self.budget.unread_fetch).await?;

        // Keep the newest unread messages within budget.
        let mut budget = self.budget.messages_tokens;
        let mut unread: Vec<&MessageView> = Vec::new();
        for m in unread_all.iter().rev() {
            let t = message_tokens(m);
            if t > budget && !unread.is_empty() {
                break;
            }
            budget = budget.saturating_sub(t);
            unread.push(m);
        }
        unread.reverse();
        let unread_omitted = unread_total - unread.len() as i64;

        // Already-read context, newest first, within the remaining budget.
        let mut recent: Vec<PayloadMessage> = Vec::new();
        if budget > 0 && cursor > 0 {
            let read = self.store.list_messages(conv.id, scope, Some(cursor + 1), self.budget.recent_fetch).await?;
            for m in read.iter().rev() {
                let t = message_tokens(m);
                if t > budget {
                    break;
                }
                budget -= t;
                recent.push(to_payload(m));
            }
            recent.reverse();
        }

        // Shared rolling summary: the conversation's, plus the thread's when woken in a thread.
        let summary = {
            let conv_summary = self.store.latest_summary(conv.id, None).await?;
            let thread_summary = match wake.thread_root_id {
                Some(r) => self.store.latest_summary(conv.id, Some(r)).await?,
                None => None,
            };
            match (conv_summary, thread_summary) {
                (None, None) => None,
                (Some(c), None) => Some(summary::clip_tokens(&c.body, self.budget.summary_tokens)),
                (None, Some(t)) => {
                    Some(summary::clip_tokens(&format!("Thread 摘要：\n{}", t.body), self.budget.summary_tokens))
                }
                (Some(c), Some(t)) => Some(summary::clip_tokens(
                    &format!("對話摘要：\n{}\n\nThread 摘要：\n{}", c.body, t.body),
                    self.budget.summary_tokens,
                )),
            }
        };

        let previous_private_logs =
            self.store.recent_private_logs(agent.member.id, conv.id, self.budget.private_logs).await?;

        let also_woken = {
            let siblings = self.store.wakes_for_trigger(wake.trigger_message_id).await?;
            let ids: Vec<_> = siblings
                .iter()
                .filter(|w| w.agent_id != wake.agent_id && w.status != WakeStatus::Cancelled)
                .map(|w| w.agent_id)
                .collect();
            members.iter().filter(|m| ids.contains(&m.member_id)).map(|m| m.handle.clone()).collect()
        };

        Ok(WakePayload {
            wake_id: wake.id,
            reason: wake.reason,
            mode: wake.mode,
            conversation: PayloadConversation {
                id: conv.id,
                kind: conv.kind,
                name: conv.name.clone(),
                topic: conv.topic.clone(),
                members: members
                    .iter()
                    .map(|m| PayloadMember {
                        handle: m.handle.clone(),
                        kind: m.kind,
                        description: m.description.clone(),
                    })
                    .collect(),
            },
            thread_root_id: wake.thread_root_id,
            thread_root,
            trigger_message_id: wake.trigger_message_id,
            summary,
            recent,
            unread: unread.into_iter().map(to_payload).collect(),
            unread_omitted,
            previous_private_logs,
            discussion: None,
            also_woken,
            limits: PayloadLimits {
                depth: wake.depth,
                max_depth: self.limits.max_depth,
                deadline_seconds: agent.max_wake_seconds as i64,
            },
        })
    }
}
