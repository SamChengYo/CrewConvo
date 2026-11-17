//! Webhook delivery (spec 5.1): the platform POSTs events to the agent's URL with an
//! HMAC-SHA256 signature over `"{timestamp}.{body}"`. A 2xx response to a `wake` counts as
//! `wake.ack`; the agent reports `wake.heartbeat` / `wake.done` via `POST /agent/v1/replies`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use ac_core::model::{Agent, AgentOrigin};
use ac_core::protocol::{AgentEvent, AgentReply};
use ac_decider::net;
use ac_service::{AgentLifecycle, Service};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tokio::task::JoinHandle;
use uuid::Uuid;

pub const SIGNATURE_HEADER: &str = "x-crewconvo-signature";
pub const TIMESTAMP_HEADER: &str = "x-crewconvo-timestamp";
pub const EVENT_HEADER: &str = "x-crewconvo-event";

/// `v1=<hex>` signature of `"{timestamp}.{body}"`.
pub fn sign(secret: &str, timestamp: i64, body: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(format!("{timestamp}.{body}").as_bytes());
    let bytes = mac.finalize().into_bytes();
    format!("v1={}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// Verification helper for agent implementers (rejects stale timestamps → replay protection).
pub fn verify(secret: &str, timestamp: i64, body: &str, signature: &str, now: i64, tolerance_secs: i64) -> bool {
    if (now - timestamp).abs() > tolerance_secs {
        return false;
    }
    let expected = sign(secret, timestamp, body);
    expected.len() == signature.len()
        && expected.bytes().zip(signature.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

pub struct WebhookManager {
    service: Service,
    tasks: Mutex<HashMap<Uuid, JoinHandle<()>>>,
}

impl WebhookManager {
    pub fn new(service: Service) -> Arc<Self> {
        let m = Arc::new(WebhookManager { service: service.clone(), tasks: Mutex::new(HashMap::new()) });
        service.add_lifecycle(Arc::new(Hook(Arc::downgrade(&m))));
        m
    }

    pub async fn load_all(self: &Arc<Self>) -> ac_core::AppResult<()> {
        for a in self.service.store().list_webhook_agents().await? {
            self.sync(&a);
        }
        Ok(())
    }

    /// Starts or stops the connector according to the agent's current settings.
    pub fn sync(self: &Arc<Self>, agent: &Agent) {
        let wants = agent.origin == AgentOrigin::External
            && agent.delivery.as_deref() == Some("webhook")
            && agent.member.deactivated_at.is_none();
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(t) = tasks.remove(&agent.member.id) {
            t.abort();
        }
        if wants {
            let me = self.clone();
            let id = agent.member.id;
            tasks.insert(id, tokio::spawn(async move { me.run(id).await }));
        }
    }

    async fn run(self: Arc<Self>, agent_id: Uuid) {
        let engine = self.service.engine().clone();
        let mut link = engine.connect(agent_id);
        while let Some(ev) = link.events.recv().await {
            let event_name = match &ev {
                AgentEvent::Wake { .. } => "wake",
                AgentEvent::WakeCancelled { .. } => "wake.cancelled",
                AgentEvent::Ping => continue, // no pings over webhooks
            };
            let wake_id = match &ev {
                AgentEvent::Wake { payload } => Some(payload.wake_id),
                _ => None,
            };
            match self.deliver(agent_id, event_name, &ev).await {
                Ok(()) => {
                    if let Some(w) = wake_id {
                        let _ = link.replier.reply(AgentReply::WakeAck { wake_id: w }).await;
                    }
                }
                Err(e) => tracing::warn!(agent = %agent_id, event = event_name, error = %e, "webhook delivery failed"),
            }
        }
    }

    async fn deliver(&self, agent_id: Uuid, event_name: &str, ev: &AgentEvent) -> Result<(), String> {
        let (url, secret) =
            self.service.webhook_target(agent_id).await.map_err(|e| e.to_string())?.ok_or("webhook not configured")?;
        let body = serde_json::to_string(ev).map_err(|e| e.to_string())?;
        let mut last_err = String::new();
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
            }
            // Re-check the URL on every attempt (DNS may change); pin the checked addresses.
            let (u, addrs) = net::check_url(&url, self.service.egress()).await?;
            let client = net::pinned_client(&u, &addrs, Duration::from_secs(10))?;
            let ts = chrono::Utc::now().timestamp();
            let res = client
                .post(u)
                .header("content-type", "application/json")
                .header(TIMESTAMP_HEADER, ts.to_string())
                .header(SIGNATURE_HEADER, sign(&secret, ts, &body))
                .header(EVENT_HEADER, event_name)
                .body(body.clone())
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(r) if r.status().is_client_error() => return Err(format!("HTTP {}", r.status())),
                Ok(r) => last_err = format!("HTTP {}", r.status()),
                Err(e) => last_err = e.to_string(),
            }
        }
        Err(last_err)
    }
}

struct Hook(Weak<WebhookManager>);

impl AgentLifecycle for Hook {
    fn agent_changed(&self, agent: &Agent) {
        if let Some(m) = self.0.upgrade() {
            m.sync(agent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures() {
        let s = sign("secret", 1_700_000_000, "{\"a\":1}");
        assert!(s.starts_with("v1="));
        assert!(verify("secret", 1_700_000_000, "{\"a\":1}", &s, 1_700_000_100, 300));
        assert!(!verify("secret", 1_700_000_000, "{\"a\":2}", &s, 1_700_000_100, 300));
        assert!(!verify("secret", 1_700_000_000, "{\"a\":1}", &s, 1_700_009_999, 300), "replay window");
    }
}
