//! Observability for humans: the activity feed and the "agent view" of a wake.

use ac_core::model::Member;
use ac_core::{AppError, AppResult};
use ac_store::WakeActivity;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::Service;

impl Service {
    /// Recent wakes in conversations the actor belongs to.
    pub async fn activity(
        &self,
        actor: &Member,
        before: Option<DateTime<Utc>>,
        limit: Option<i64>,
    ) -> AppResult<Vec<WakeActivity>> {
        let mut rows = self.inner.store.wake_activity(actor.id, before, limit.unwrap_or(50).clamp(1, 200)).await?;
        if !actor.is_admin {
            for r in &mut rows {
                r.wake.private_log = None;
            }
        }
        Ok(rows)
    }

    /// Exactly what the agent received for a wake. Private logs are only shown to admins.
    pub async fn wake_payload(&self, actor: &Member, wake_id: Uuid) -> AppResult<Value> {
        let wake = self.inner.store.get_wake(wake_id).await?.ok_or_else(|| AppError::not_found("wake"))?;
        self.member_conversation(actor, wake.conversation_id).await?;
        let mut payload = self
            .inner
            .store
            .wake_payload(wake_id)
            .await?
            .ok_or_else(|| AppError::not_found("這次喚醒還沒有送出 payload（可能仍在排隊、或已在送出前被取消）"))?;
        if !actor.is_admin {
            if let Some(o) = payload.as_object_mut() {
                o.insert("previous_private_logs".into(), Value::String("（僅管理員可見）".into()));
            }
        }
        Ok(payload)
    }
}
