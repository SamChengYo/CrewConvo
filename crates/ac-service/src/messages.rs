use ac_core::bus::RealtimeEvent;
use ac_core::mention::{MENTION_ALL, parse_mentions, resolve_mention};
use ac_core::model::{Member, MemberKind, MessageView, Wake};
use ac_core::{AppError, AppResult};
use ac_store::{MessageScope, NewMessage};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::Service;

pub const MAX_BODY_CHARS: usize = 20_000;
pub const MAX_HISTORY_LIMIT: i64 = 100;

#[derive(Debug, Clone, Serialize)]
pub struct PostResult {
    pub message: MessageView,
    /// `@handle`s that were not resolved to a conversation member (not woken).
    pub ignored_mentions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WakeExplanation {
    pub message_id: Uuid,
    pub wakes: Vec<WakeWithAgent>,
    /// Rule-layer / Decider records for this message (scores, attempts, latency).
    pub decisions: Vec<ac_core::model::DecisionRecord>,
    /// The wake during which this message was posted (if an agent wrote it in a wake).
    pub produced_by: Option<WakeWithAgent>,
    /// Handles of the agents referenced in the decisions.
    pub agent_handles: std::collections::HashMap<Uuid, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WakeWithAgent {
    #[serde(flatten)]
    pub wake: Wake,
    pub agent_handle: String,
}

impl Service {
    /// The single way to post a message. `wake_id` binds the post to a running wake
    /// (sets depth / chain); without it an agent post is "proactive".
    pub async fn post_message(
        &self,
        actor: &Member,
        wake_id: Option<Uuid>,
        conversation_id: Uuid,
        body: &str,
        thread_root_id: Option<Uuid>,
    ) -> AppResult<PostResult> {
        let store = &self.inner.store;
        let body = body.trim();
        if body.is_empty() {
            return Err(AppError::bad_request("訊息內容不能是空的"));
        }
        if body.chars().count() > MAX_BODY_CHARS {
            return Err(AppError::bad_request(format!("訊息不能超過 {MAX_BODY_CHARS} 個字元")));
        }
        if actor.kind == MemberKind::System {
            return Err(AppError::forbidden("system"));
        }
        let conv = self.member_conversation(actor, conversation_id).await?;
        if conv.archived_at.is_some() {
            return Err(AppError::bad_request("對話已封存"));
        }

        // Threads hang off a top-level message of the same conversation.
        let thread_root_id = match thread_root_id {
            Some(root) => {
                let m = store.get_message(root).await?.ok_or_else(|| AppError::not_found("thread root"))?;
                if m.conversation_id != conv.id || m.deleted_at.is_some() {
                    return Err(AppError::bad_request("thread_root_id 不屬於此對話"));
                }
                Some(m.thread_root_id.unwrap_or(m.id))
            }
            None => None,
        };

        // Causal depth (spec 6.6).
        let (depth, chain_root_id, wake_id) = match (actor.kind, wake_id) {
            (MemberKind::Agent, Some(wid)) => {
                let wake = self.inner.engine.active_wake_for(wid, actor.id).await?;
                (wake.depth + 1, Some(wake.chain_root_id), Some(wake.id))
            }
            // No explicit wake: bind implicitly to the wake the agent is working on (one at a time).
            (MemberKind::Agent, None) if store.inflight_wake_for_agent(actor.id).await?.is_some() => {
                let wake = store.inflight_wake_for_agent(actor.id).await?.expect("checked");
                (wake.depth + 1, Some(wake.chain_root_id), Some(wake.id))
            }
            (MemberKind::Agent, None) => {
                let since = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
                let n = store.count_proactive_posts_since(actor.id, since).await?;
                if n >= self.inner.limits.proactive_posts_per_day {
                    return Err(AppError::RateLimited("今日主動發言次數已達上限".into()));
                }
                (0, None, None)
            }
            _ => (0, None, None),
        };

        // Server-side mention resolution; only conversation members count.
        let handles = parse_mentions(body);
        let mut mentions = Vec::new();
        let mut ignored = Vec::new();
        if !handles.is_empty() {
            let conv_members = store.conversation_members(conv.id).await?;
            let lowered: Vec<(String, uuid::Uuid)> =
                conv_members.iter().map(|cm| (cm.handle.to_lowercase(), cm.member_id)).collect();
            for h in &handles {
                if h == MENTION_ALL {
                    // @all: every other member of the conversation. Only humans can use it, so agents
                    // cannot fan out to everyone (and loop) on their own.
                    if actor.kind == MemberKind::Human {
                        for cm in &conv_members {
                            if cm.member_id != actor.id
                                && cm.kind != MemberKind::System
                                && !mentions.contains(&cm.member_id)
                            {
                                mentions.push(cm.member_id);
                            }
                        }
                    } else {
                        ignored.push(h.clone());
                    }
                    continue;
                }
                // Only conversation members count; a name glued to the following text still matches.
                match resolve_mention(h, lowered.iter().map(|(l, _)| l.as_str())) {
                    Some(hit) => {
                        let id = lowered.iter().find(|(l, _)| l == hit).map(|(_, id)| *id);
                        if let Some(id) = id
                            && !mentions.contains(&id)
                        {
                            mentions.push(id)
                        }
                    }
                    None => ignored.push(h.clone()),
                }
            }
        }

        let (msg, view) = self
            .inner
            .engine
            .insert_and_publish(&NewMessage {
                conversation_id: conv.id,
                thread_root_id,
                author_id: actor.id,
                body: body.to_string(),
                mentions,
                depth,
                chain_root_id,
                wake_id,
            })
            .await?;
        if let Some(wid) = wake_id {
            store.increment_posted(wid).await?;
        }
        if let Some(root) = thread_root_id {
            // Reply counts changed on the root.
            if let Some(root_view) = store.get_message_view(root).await? {
                self.inner.engine.bus().publish(RealtimeEvent::MessageCreated { message: root_view });
            }
        }

        if !ignored.is_empty() && actor.kind == MemberKind::Human {
            let list = ignored.iter().map(|h| format!("@{h}")).collect::<Vec<_>>().join("、");
            self.inner
                .engine
                .post_system(
                    conv.id,
                    thread_root_id,
                    format!("{list} 不是此對話的成員（或不存在），沒有被通知。可先邀請再 tag。"),
                )
                .await?;
        }

        self.inner.engine.submit(msg);
        Ok(PostResult { message: view, ignored_mentions: ignored })
    }

    /// Reactions never trigger a wake.
    pub async fn react(&self, actor: &Member, message_id: Uuid, emoji: &str, add: bool) -> AppResult<MessageView> {
        let emoji = emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 32 {
            return Err(AppError::bad_request("emoji 格式不正確"));
        }
        let store = &self.inner.store;
        let msg = store.get_message(message_id).await?.ok_or_else(|| AppError::not_found("message"))?;
        if msg.deleted_at.is_some() {
            return Err(AppError::not_found("message"));
        }
        self.member_conversation(actor, msg.conversation_id).await?;
        if add {
            store.add_reaction(message_id, actor.id, emoji).await?;
        } else {
            store.remove_reaction(message_id, actor.id, emoji).await?;
        }
        let reactions = store.reaction_summary(message_id).await?;
        self.inner.engine.bus().publish(RealtimeEvent::ReactionsChanged {
            conversation_id: msg.conversation_id,
            message_id,
            reactions,
        });
        store.get_message_view(message_id).await?.ok_or_else(|| AppError::not_found("message"))
    }

    pub async fn read_history(
        &self,
        actor: &Member,
        conversation_id: Uuid,
        thread_root_id: Option<Uuid>,
        before_seq: Option<i64>,
        limit: Option<i64>,
    ) -> AppResult<Vec<MessageView>> {
        self.member_conversation(actor, conversation_id).await?;
        let limit = limit.unwrap_or(50).clamp(1, MAX_HISTORY_LIMIT);
        self.inner
            .store
            .list_messages(conversation_id, MessageScope::from_thread(thread_root_id), before_seq, limit)
            .await
    }

    /// Marks the conversation as read up to `seq` (humans; agents advance on wake ack).
    pub async fn mark_read(&self, actor: &Member, conversation_id: Uuid, seq: i64) -> AppResult<()> {
        self.member_conversation(actor, conversation_id).await?;
        self.inner.store.update_cursor(conversation_id, actor.id, seq).await
    }

    pub async fn delete_message(&self, actor: &Member, message_id: Uuid) -> AppResult<()> {
        let store = &self.inner.store;
        let msg = store.get_message(message_id).await?.ok_or_else(|| AppError::not_found("message"))?;
        self.member_conversation(actor, msg.conversation_id).await?;
        if msg.author_id != actor.id && !actor.is_admin {
            return Err(AppError::forbidden("只能刪除自己的訊息"));
        }
        if store.soft_delete_message(message_id).await? {
            self.inner
                .engine
                .bus()
                .publish(RealtimeEvent::MessageDeleted { conversation_id: msg.conversation_id, message_id });
        }
        Ok(())
    }

    /// "Why were these agents woken?" (spec 10).
    pub async fn explain_wakes(&self, actor: &Member, message_id: Uuid) -> AppResult<WakeExplanation> {
        let store = &self.inner.store;
        let msg = store.get_message(message_id).await?.ok_or_else(|| AppError::not_found("message"))?;
        self.member_conversation(actor, msg.conversation_id).await?;
        let wakes = store.wakes_for_trigger(message_id).await?;
        let produced = store.wake_by_id_opt(msg.wake_id).await?;
        let mut decisions = store.decisions_for_message(message_id).await?;
        let mut ids: Vec<Uuid> = wakes.iter().map(|w| w.agent_id).collect();
        ids.extend(produced.iter().map(|w| w.agent_id));
        for d in &decisions {
            ids.extend(d.woken.iter().copied());
        }
        ids.sort();
        ids.dedup();
        let agents = store.get_members(&ids).await?;
        let handle = |id: Uuid| agents.iter().find(|a| a.id == id).map(|a| a.handle.clone()).unwrap_or_default();
        let redact = |mut w: ac_core::model::Wake| {
            // private_log is the agent's own memory; only admins may inspect it.
            if !actor.is_admin {
                w.private_log = None;
            }
            WakeWithAgent { agent_handle: handle(w.agent_id), wake: w }
        };
        if !actor.is_admin {
            // Raw provider payloads are for admins.
            for d in &mut decisions {
                d.request = None;
                d.response = None;
            }
        }
        Ok(WakeExplanation {
            message_id,
            wakes: wakes.into_iter().map(redact).collect(),
            produced_by: produced.map(redact),
            agent_handles: agents.iter().map(|a| (a.id, a.handle.clone())).collect(),
            decisions,
        })
    }
}
