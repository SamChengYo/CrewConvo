use std::time::Duration;

use ac_core::bus::RealtimeEvent;
use ac_core::model::{Conversation, ConversationKind, ConversationMemberView, Member, MemberKind, Visibility, Wake};
use ac_core::{AppError, AppResult};
use ac_store::ConversationListItem;
use serde::Serialize;
use uuid::Uuid;

use crate::{Service, require_human};

#[derive(Debug, Clone, Serialize)]
pub struct ConversationDetail {
    #[serde(flatten)]
    pub conversation: Conversation,
    pub members: Vec<ConversationMemberView>,
    pub is_member: bool,
    pub active_wakes: Vec<Wake>,
}

fn validate_channel_name(name: &str) -> AppResult<String> {
    let n = name.trim().trim_start_matches('#').trim();
    if n.is_empty() || n.chars().count() > 80 {
        return Err(AppError::bad_request("頻道名稱需為 1–80 個字元"));
    }
    if n.chars().any(|c| c.is_whitespace() || c == '@' || c == '#') {
        return Err(AppError::bad_request("頻道名稱不能包含空白、@ 或 #"));
    }
    Ok(n.to_string())
}

impl Service {
    pub async fn list_conversations(&self, actor: &Member) -> AppResult<Vec<ConversationListItem>> {
        self.inner.store.list_conversations_for(actor.id).await
    }

    pub async fn list_public_channels(&self, actor: &Member) -> AppResult<Vec<Conversation>> {
        self.inner.store.list_public_channels(actor.workspace_id).await
    }

    pub async fn conversation_detail(&self, actor: &Member, id: Uuid) -> AppResult<ConversationDetail> {
        let (conv, is_member) = self.visible_conversation(actor, id).await?;
        if !is_member && conv.visibility == Visibility::Private {
            return Err(AppError::not_found("conversation"));
        }
        if !is_member && conv.kind == ConversationKind::Dm {
            return Err(AppError::not_found("conversation"));
        }
        Ok(ConversationDetail {
            members: self.inner.store.conversation_members(id).await?,
            active_wakes: if is_member { self.inner.engine.active_wakes(id).await? } else { vec![] },
            conversation: conv,
            is_member,
        })
    }

    async fn resolve_workspace_members(&self, actor: &Member, ids: &[Uuid]) -> AppResult<Vec<Member>> {
        let mut uniq: Vec<Uuid> = ids.to_vec();
        uniq.sort();
        uniq.dedup();
        let members = self.inner.store.get_members(&uniq).await?;
        if members.len() != uniq.len()
            || members.iter().any(|m| {
                m.workspace_id != actor.workspace_id || m.kind == MemberKind::System || m.deactivated_at.is_some()
            })
        {
            return Err(AppError::bad_request("包含不存在或無法邀請的成員"));
        }
        Ok(members)
    }

    pub async fn create_channel(
        &self,
        actor: &Member,
        name: &str,
        topic: Option<&str>,
        visibility: Visibility,
        invite: &[Uuid],
    ) -> AppResult<Conversation> {
        if actor.kind == MemberKind::Agent
            && !self.inner.rate.check(
                actor.id,
                "create_channel",
                self.inner.limits.create_channel_per_hour,
                Duration::from_secs(3600),
            )
        {
            return Err(AppError::RateLimited("create_channel 每小時次數已達上限".into()));
        }
        let name = validate_channel_name(name)?;
        let invited = self.resolve_workspace_members(actor, invite).await?;
        let mut members = vec![actor.id];
        members.extend(invited.iter().map(|m| m.id));
        let conv = self
            .inner
            .store
            .create_conversation(
                actor.workspace_id,
                ConversationKind::Channel,
                Some(&name),
                topic.map(str::trim).filter(|t| !t.is_empty()),
                visibility,
                actor.id,
                &members,
            )
            .await
            .map_err(|e| match e {
                AppError::Conflict(_) => AppError::Conflict(format!("頻道 #{name} 已存在")),
                other => other,
            })?;
        for m in &members {
            self.inner.engine.bus().publish(RealtimeEvent::MembershipChanged {
                conversation_id: conv.id,
                member_id: *m,
                joined: true,
            });
        }
        Ok(conv)
    }

    /// Gets or creates the DM whose members are the actor plus `member_ids`.
    pub async fn open_dm(&self, actor: &Member, member_ids: &[Uuid]) -> AppResult<Conversation> {
        let others: Vec<Uuid> = member_ids.iter().copied().filter(|id| *id != actor.id).collect();
        if others.is_empty() {
            return Err(AppError::bad_request("私訊至少需要另一位成員"));
        }
        self.resolve_workspace_members(actor, &others).await?;
        let mut all = others.clone();
        all.push(actor.id);
        all.sort();
        all.dedup();
        if let Some(c) = self.inner.store.find_dm(actor.workspace_id, &all).await? {
            return Ok(c);
        }
        let conv = self
            .inner
            .store
            .create_conversation(
                actor.workspace_id,
                ConversationKind::Dm,
                None,
                None,
                Visibility::Private,
                actor.id,
                &all,
            )
            .await?;
        for m in &all {
            self.inner.engine.bus().publish(RealtimeEvent::MembershipChanged {
                conversation_id: conv.id,
                member_id: *m,
                joined: true,
            });
        }
        Ok(conv)
    }

    pub async fn invite(&self, actor: &Member, conversation_id: Uuid, member_ids: &[Uuid]) -> AppResult<Vec<Uuid>> {
        let conv = self.member_conversation(actor, conversation_id).await?;
        if conv.kind == ConversationKind::Dm {
            return Err(AppError::bad_request("私訊的成員固定，請另開一個私訊"));
        }
        if actor.kind == MemberKind::Agent
            && !self.inner.rate.check(actor.id, "invite", self.inner.limits.invite_per_hour, Duration::from_secs(3600))
        {
            return Err(AppError::RateLimited("invite 每小時次數已達上限".into()));
        }
        let members = self.resolve_workspace_members(actor, member_ids).await?;
        let mut added = Vec::new();
        for m in members {
            if self.inner.store.add_conversation_member(conv.id, m.id, Some(actor.id)).await? {
                self.inner.engine.bus().publish(RealtimeEvent::MembershipChanged {
                    conversation_id: conv.id,
                    member_id: m.id,
                    joined: true,
                });
                added.push(m.id);
            }
        }
        Ok(added)
    }

    /// Joins a public channel.
    pub async fn join(&self, actor: &Member, conversation_id: Uuid) -> AppResult<()> {
        let (conv, is_member) = self.visible_conversation(actor, conversation_id).await?;
        if is_member {
            return Ok(());
        }
        if conv.kind != ConversationKind::Channel || conv.visibility != Visibility::Public {
            return Err(AppError::forbidden("私有對話需要被邀請才能加入"));
        }
        if self.inner.store.add_conversation_member(conv.id, actor.id, None).await? {
            self.inner.engine.bus().publish(RealtimeEvent::MembershipChanged {
                conversation_id: conv.id,
                member_id: actor.id,
                joined: true,
            });
        }
        Ok(())
    }

    pub async fn leave(&self, actor: &Member, conversation_id: Uuid) -> AppResult<()> {
        self.member_conversation(actor, conversation_id).await?;
        if self.inner.store.remove_conversation_member(conversation_id, actor.id).await? {
            self.inner.engine.bus().publish(RealtimeEvent::MembershipChanged {
                conversation_id,
                member_id: actor.id,
                joined: false,
            });
        }
        Ok(())
    }

    /// Gives a channel another name. Members only; direct messages have no name.
    pub async fn rename_channel(&self, actor: &Member, conversation_id: Uuid, name: &str) -> AppResult<Conversation> {
        require_human(actor)?;
        let conv = self.member_conversation(actor, conversation_id).await?;
        if conv.kind != ConversationKind::Channel {
            return Err(AppError::bad_request("私訊沒有名稱可以改"));
        }
        let name = validate_channel_name(name)?;
        let renamed = self.inner.store.rename_conversation(conversation_id, &name).await.map_err(|e| match e {
            AppError::Conflict(_) => AppError::Conflict(format!("頻道 #{name} 已存在")),
            other => other,
        })?;
        self.inner.engine.bus().publish(RealtimeEvent::ConversationUpdated { conversation: renamed.clone() });
        Ok(renamed)
    }

    /// Archives a channel: it disappears from the lists and takes no more messages; its
    /// history stays in the database. Pending work for agents in it is cancelled first.
    pub async fn archive_channel(&self, actor: &Member, conversation_id: Uuid) -> AppResult<()> {
        require_human(actor)?;
        let conv = self.member_conversation(actor, conversation_id).await?;
        if conv.kind != ConversationKind::Channel {
            return Err(AppError::bad_request("只有頻道可以封存"));
        }
        if conv.name.as_deref() == Some("general") {
            return Err(AppError::bad_request("#general 是所有人的頻道，不能封存"));
        }
        let members = self.inner.store.conversation_members(conversation_id).await?;
        self.inner.engine.stop_conversation(conversation_id).await?;
        if self.inner.store.archive_conversation(conversation_id).await?.is_some() {
            // Everybody's list of conversations has one fewer.
            for m in members {
                self.inner.engine.bus().publish(RealtimeEvent::MembershipChanged {
                    conversation_id,
                    member_id: m.member_id,
                    joined: false,
                });
            }
        }
        Ok(())
    }

    /// Human "stop" button (spec 6.7).
    pub async fn stop_conversation(&self, actor: &Member, conversation_id: Uuid) -> AppResult<Vec<Wake>> {
        require_human(actor)?;
        self.member_conversation(actor, conversation_id).await?;
        let cancelled = self.inner.engine.stop_conversation(conversation_id).await?;
        self.inner
            .engine
            .post_system(
                conversation_id,
                None,
                format!(
                    "@{} 按下了停止：已取消 {} 個進行中的喚醒。Agent 的訊息不會再觸發喚醒，直到有人類發言。",
                    actor.handle,
                    cancelled.len()
                ),
            )
            .await?;
        Ok(cancelled)
    }
}

/// One entry of the channel directory (`list_channels`).
#[derive(Debug, Clone, Serialize)]
pub struct ChannelInfo {
    pub id: Uuid,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub visibility: Visibility,
    pub joined: bool,
    pub member_count: usize,
    pub agents: Vec<String>,
    pub humans: Vec<String>,
}

/// The lobby: the one channel every agent can see and join without an invite.
pub const LOBBY_CHANNEL: &str = "general";

pub(crate) fn is_lobby(c: &Conversation) -> bool {
    c.kind == ConversationKind::Channel
        && c.visibility == Visibility::Public
        && c.name.as_deref() == Some(LOBBY_CHANNEL)
}

impl Service {
    /// Humans: public channels plus the private channels they are in.
    /// Agents: only the channels they are in, plus the lobby (#general).
    pub async fn channel_directory(&self, actor: &Member, query: Option<&str>) -> AppResult<Vec<ChannelInfo>> {
        let store = &self.inner.store;
        let mine = store.list_conversations_for(actor.id).await?;
        let mut convs: Vec<Conversation> = store.list_public_channels(actor.workspace_id).await?;
        if actor.kind == MemberKind::Agent {
            convs.retain(is_lobby);
        }
        for c in &mine {
            if c.conversation.kind == ConversationKind::Channel && !convs.iter().any(|x| x.id == c.conversation.id) {
                convs.push(c.conversation.clone());
            }
        }
        let q = query.map(|q| q.trim().trim_start_matches('#').to_lowercase()).filter(|q| !q.is_empty());
        let mut out = Vec::new();
        for c in convs {
            if let Some(q) = &q {
                let hay = format!("{} {}", c.name.clone().unwrap_or_default(), c.topic.clone().unwrap_or_default())
                    .to_lowercase();
                if !hay.contains(q) {
                    continue;
                }
            }
            let members = store.conversation_members(c.id).await?;
            out.push(ChannelInfo {
                id: c.id,
                joined: members.iter().any(|m| m.member_id == actor.id),
                member_count: members.len(),
                agents: members.iter().filter(|m| m.kind == MemberKind::Agent).map(|m| m.handle.clone()).collect(),
                humans: members.iter().filter(|m| m.kind == MemberKind::Human).map(|m| m.handle.clone()).collect(),
                name: c.name,
                topic: c.topic,
                visibility: c.visibility,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Joins a channel given its id or name (`#name`). Humans can join any public channel;
    /// agents only the lobby — other channels need an invite.
    pub async fn join_channel_ref(&self, actor: &Member, channel: &str) -> AppResult<Conversation> {
        let r = channel.trim();
        let agent = actor.kind == MemberKind::Agent;
        let not_found = |what: &str| {
            if agent {
                AppError::not_found(format!(
                    "找不到頻道 {what}（Agent 只能自行加入 #{LOBBY_CHANNEL}，其他頻道需要被邀請）"
                ))
            } else {
                AppError::not_found(format!("找不到公開頻道 {what}"))
            }
        };
        let id = match r.parse::<Uuid>() {
            Ok(id) => id,
            Err(_) => {
                let name = r.trim_start_matches('#').to_lowercase();
                self.inner
                    .store
                    .list_public_channels(actor.workspace_id)
                    .await?
                    .into_iter()
                    .find(|c| c.name.as_deref().map(|n| n.to_lowercase() == name).unwrap_or(false))
                    .map(|c| c.id)
                    .ok_or_else(|| not_found(&format!("#{name}")))?
            }
        };
        if agent {
            let (conv, is_member) = self.visible_conversation(actor, id).await?;
            if !is_member && !is_lobby(&conv) {
                return Err(not_found(r));
            }
        }
        self.join(actor, id).await?;
        self.inner.store.get_conversation(id).await?.ok_or_else(|| AppError::not_found("conversation"))
    }
}
