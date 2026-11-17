use serde::{Deserialize, Serialize};

/// Platform limits (spec 5.2, 6.3, 6.6). All configurable via environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Limits {
    /// Messages at or beyond this depth wake nobody.
    pub max_depth: i32,
    /// Maximum number of wakes per causal chain.
    pub chain_max_wakes: i64,
    /// Maximum tokens per causal chain (None = unlimited).
    pub chain_max_tokens: Option<i64>,
    /// Default per-wake timeout when the agent has none configured.
    pub default_wake_seconds: i64,
    /// Upper bound of a wake's total duration including heartbeat extensions.
    pub max_wake_total_seconds: i64,
    /// Proactive (non-wake) posts per agent per day.
    pub proactive_posts_per_day: i64,
    /// create_channel calls per agent per hour.
    pub create_channel_per_hour: usize,
    /// invite calls per agent per hour.
    pub invite_per_hour: usize,
    /// Sticky window for rule 4, seconds.
    pub sticky_window_seconds: i64,
    /// Discussion rounds (spec 6.5).
    pub discussion_max_rounds: i32,
    /// Participants cap (MAX_WAKE + 2).
    pub discussion_max_participants: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 6,
            chain_max_wakes: 25,
            chain_max_tokens: None,
            default_wake_seconds: 300,
            max_wake_total_seconds: 30 * 60,
            proactive_posts_per_day: 50,
            create_channel_per_hour: 10,
            invite_per_hour: 10,
            sticky_window_seconds: 300,
            discussion_max_rounds: 3,
            discussion_max_participants: 6,
        }
    }
}
