//! Per-agent wake queue (spec 6.3): one wake at a time per agent.

use std::time::Duration;

use ac_core::model::{AgentStatus, Wake, WakeStatus};
use ac_core::protocol::{AgentEvent, AgentReply};
use ac_core::{AgentId, AppResult};
use chrono::Utc;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::{WakeEngine, WakeSignal, start_of_day};

impl WakeEngine {
    /// Makes sure a worker task is draining this agent's pending wakes.
    pub async fn ensure_worker(&self, agent_id: AgentId) {
        let mut workers = self.inner.workers.lock().await;
        if workers.insert(agent_id) {
            let e = self.clone();
            tokio::spawn(async move { e.worker_loop(agent_id).await });
        }
    }

    async fn worker_loop(&self, agent_id: AgentId) {
        loop {
            let next = match self.inner.store.next_pending_wake(agent_id).await {
                Ok(w) => w,
                Err(err) => {
                    tracing::error!(%agent_id, error = %err, "worker failed to load next wake");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            match next {
                Some(wake) => {
                    let id = wake.id;
                    let in_discussion = wake.discussion_id.is_some();
                    if let Err(err) = self.run_wake(wake).await {
                        tracing::error!(wake_id = %id, error = %err, "wake failed");
                        let _ = self.finish_and_publish(id, WakeStatus::Failed, None, None).await;
                    }
                    if in_discussion && let Err(err) = self.on_discussion_wake_finished(id).await {
                        tracing::error!(wake_id = %id, error = %err, "discussion advance failed");
                    }
                }
                None => {
                    // Exit only if nothing was queued while we were checking.
                    let mut workers = self.inner.workers.lock().await;
                    match self.inner.store.next_pending_wake(agent_id).await {
                        Ok(None) => {
                            workers.remove(&agent_id);
                            return;
                        }
                        Ok(Some(_)) => continue,
                        Err(_) => {
                            workers.remove(&agent_id);
                            return;
                        }
                    }
                }
            }
        }
    }

    async fn fail_with_notice(&self, wake: &Wake, notice: String) -> AppResult<()> {
        if let Some(w) = self.inner.store.transition_wake(wake.id, &[WakeStatus::Pending], WakeStatus::Failed).await? {
            self.publish_wake(&w).await;
            self.post_system(wake.conversation_id, wake.thread_root_id, notice).await?;
        }
        Ok(())
    }

    async fn run_wake(&self, wake: Wake) -> AppResult<()> {
        let store = &self.inner.store;
        let Some(agent) = store.get_agent(wake.agent_id).await? else {
            store.transition_wake(wake.id, &[WakeStatus::Pending], WakeStatus::Failed).await?;
            return Ok(());
        };
        let handle = agent.member.handle.clone();

        if matches!(agent.status, AgentStatus::Offline | AgentStatus::Disabled) || agent.member.deactivated_at.is_some()
        {
            return self
                .fail_with_notice(&wake, format!("@{handle} 目前為 {} 狀態，這次喚醒未執行。", agent.status))
                .await;
        }
        if !self.is_connected(agent.member.id) {
            return self.fail_with_notice(&wake, format!("@{handle} 目前未連線，這次喚醒未執行。")).await;
        }
        if let Some(budget) = agent.daily_token_budget
            && store.agent_tokens_since(agent.member.id, start_of_day(Utc::now())).await? >= budget
        {
            return self.fail_with_notice(&wake, format!("@{handle} 今日的 token 預算已用完，這次喚醒未執行。")).await;
        }
        if let Some(cap) = self.inner.limits.chain_max_tokens {
            let (_, tokens) = store.chain_stats(wake.chain_root_id).await?;
            if tokens >= cap {
                if let Some(w) = store.transition_wake(wake.id, &[WakeStatus::Pending], WakeStatus::Cancelled).await? {
                    self.publish_wake(&w).await;
                }
                return Ok(());
            }
        }

        // Payload is assembled at send time so it contains every unread message (merge semantics).
        let mut payload = self.inner.context.assemble(&wake, &agent).await?;
        payload.discussion = self.discussion_payload(&wake, &agent).await?;
        // Snapshot of what the agent sees, for the "agent view" in the UI.
        if let Ok(v) = serde_json::to_value(&payload) {
            let _ = store.set_wake_payload(wake.id, &v).await;
        }
        let max_seq = payload.max_seq();

        let (tx, mut rx) = mpsc::unbounded_channel::<WakeSignal>();
        self.inner.inflight.lock().unwrap().insert(wake.id, tx);
        let _guard = InflightGuard { engine: self.clone(), wake_id: wake.id };

        let Some(delivered) = store.transition_wake(wake.id, &[WakeStatus::Pending], WakeStatus::Delivered).await?
        else {
            return Ok(()); // cancelled meanwhile
        };
        self.publish_wake(&delivered).await;

        if !self.send_event(agent.member.id, AgentEvent::Wake { payload: Box::new(payload) }) {
            self.finish_and_publish(wake.id, WakeStatus::Failed, None, None).await?;
            self.post_system(
                wake.conversation_id,
                wake.thread_root_id,
                format!("@{handle} 連線中斷，這次喚醒未完成。"),
            )
            .await?;
            return Ok(());
        }

        let step = Duration::from_secs(agent.max_wake_seconds.max(1) as u64);
        let started = Instant::now();
        let hard_deadline = started + Duration::from_secs(self.inner.limits.max_wake_total_seconds.max(1) as u64);
        let mut deadline = (started + step).min(hard_deadline);
        let mut running = false;

        loop {
            tokio::select! {
                sig = rx.recv() => match sig {
                    Some(WakeSignal::Reply(AgentReply::WakeAck { .. })) => {
                        if !running {
                            running = true;
                            store.update_cursor(wake.conversation_id, agent.member.id, max_seq).await?;
                            if let Some(w) = store.transition_wake(wake.id, &[WakeStatus::Delivered], WakeStatus::Running).await? {
                                self.publish_wake(&w).await;
                            }
                        }
                    }
                    Some(WakeSignal::Reply(AgentReply::WakeHeartbeat { .. })) => {
                        deadline = (Instant::now() + step).min(hard_deadline);
                    }
                    Some(WakeSignal::Reply(AgentReply::WakeDone { private_log, tokens_used, .. })) => {
                        if !running {
                            store.update_cursor(wake.conversation_id, agent.member.id, max_seq).await?;
                        }
                        if self.finish_and_publish(wake.id, WakeStatus::Done, private_log.as_deref(), tokens_used).await?.is_none() {
                            // Already terminal (e.g. cancelled concurrently): still account tokens.
                            if let Some(t) = tokens_used.filter(|t| *t > 0) {
                                store.add_wake_tokens(wake.id, t).await?;
                            }
                        }
                        return Ok(());
                    }
                    Some(WakeSignal::Cancel) | None => {
                        self.send_event(agent.member.id, AgentEvent::WakeCancelled { wake_id: wake.id });
                        return Ok(());
                    }
                },
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::warn!(wake_id = %wake.id, agent = %handle, "wake timed out");
                    self.send_event(agent.member.id, AgentEvent::WakeCancelled { wake_id: wake.id });
                    self.finish_and_publish(wake.id, WakeStatus::Timeout, None, None).await?;
                    return Ok(());
                }
            }
        }
    }
}

struct InflightGuard {
    engine: WakeEngine,
    wake_id: uuid::Uuid,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.engine.inner.inflight.lock().unwrap().remove(&self.wake_id);
    }
}

impl WakeEngine {
    /// Fire-and-forget `ensure_worker` (breaks the async recursion worker → discussion → worker).
    pub(crate) fn kick_worker(&self, agent_id: AgentId) {
        let e = self.clone();
        let fut: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
            Box::pin(async move { e.ensure_worker(agent_id).await });
        tokio::spawn(fut);
    }
}
