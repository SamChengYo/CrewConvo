//! Long-poll delivery: `GET /agent/v1/events?timeout=25` blocks until the platform has an
//! event for the agent (or the timeout passes). The simplest way for CLI-style agents to take
//! part — no WebSocket client, no public webhook URL. Replies go to `POST /agent/v1/replies`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ac_core::AgentId;
use ac_core::protocol::AgentEvent;
use ac_wake::AgentLink;
use tokio::sync::Mutex as AsyncMutex;

/// Considered offline after this long without a poll.
pub const IDLE: Duration = Duration::from_secs(90);

struct Poller {
    link: Arc<AsyncMutex<AgentLink>>,
    last_poll: Instant,
}

#[derive(Clone, Default)]
pub struct PollHub {
    pollers: Arc<Mutex<HashMap<AgentId, Poller>>>,
}

impl PollHub {
    /// Returns the agent's link, connecting it if needed (`true` when newly connected).
    pub fn link(&self, engine: &ac_wake::WakeEngine, agent: AgentId) -> (Arc<AsyncMutex<AgentLink>>, bool) {
        let mut map = self.pollers.lock().unwrap();
        if let Some(p) = map.get_mut(&agent) {
            if engine.is_connected(agent) {
                p.last_poll = Instant::now();
                return (p.link.clone(), false);
            }
        }
        let link = Arc::new(AsyncMutex::new(engine.connect(agent)));
        map.insert(agent, Poller { link: link.clone(), last_poll: Instant::now() });
        (link, true)
    }

    pub fn touch(&self, agent: AgentId) {
        if let Some(p) = self.pollers.lock().unwrap().get_mut(&agent) {
            p.last_poll = Instant::now();
        }
    }

    /// Removes pollers idle for longer than [`IDLE`]; returns their agent ids.
    pub fn reap(&self) -> Vec<AgentId> {
        let mut map = self.pollers.lock().unwrap();
        let idle: Vec<AgentId> = map.iter().filter(|(_, p)| p.last_poll.elapsed() > IDLE).map(|(a, _)| *a).collect();
        for a in &idle {
            if let Some(p) = map.remove(a) {
                if let Ok(l) = p.link.try_lock() {
                    l.replier.disconnect();
                }
            }
        }
        idle
    }

    /// Waits up to `timeout` for events; returns every event that is ready.
    pub async fn wait(link: &Arc<AsyncMutex<AgentLink>>, timeout: Duration) -> Option<Vec<AgentEvent>> {
        let mut l = link.lock().await;
        let first = match tokio::time::timeout(timeout, l.events.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => return None, // connection replaced or revoked
            Err(_) => return Some(vec![]),
        };
        let mut out = vec![first];
        while let Ok(ev) = l.events.try_recv() {
            out.push(ev);
        }
        Some(out.into_iter().filter(|e| !matches!(e, AgentEvent::Ping)).collect())
    }
}
