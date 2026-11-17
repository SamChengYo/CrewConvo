use std::path::PathBuf;

use ac_context::SummaryConfig;
use ac_core::limits::Limits;
use ac_decider::EgressPolicy;
use ac_store::DbConfig;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: String,
    pub db: DbConfig,
    pub db_max_connections: u32,
    pub web_dist: Option<PathBuf>,
    pub cookie_secure: bool,
    pub limits: Limits,
    /// Load all agents, recover wakes, rebuild the search index at startup.
    /// Disabled in tests, where several app instances share one database.
    pub manage_existing: bool,
    /// None → in-memory search index.
    pub search_index_dir: Option<PathBuf>,
    pub summary: SummaryConfig,
    /// Base64 32-byte key for secrets at rest. None → ephemeral key (dev/test only).
    pub master_key: Option<String>,
    pub egress: EgressPolicy,
    /// Enable the Docker sandbox.
    pub sandbox: bool,
    /// Public URL other platforms use (PUBLIC_BASE_URL); None → derived from the request.
    pub public_base_url: Option<String>,
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

fn env_flag(key: &str, default: bool) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let mut limits = Limits::default();
        if let Some(v) = env_parse("CC_MAX_DEPTH") {
            limits.max_depth = v;
        }
        if let Some(v) = env_parse("CC_CHAIN_MAX_WAKES") {
            limits.chain_max_wakes = v;
        }
        if let Some(v) = env_parse::<i64>("CC_CHAIN_MAX_TOKENS") {
            limits.chain_max_tokens = (v > 0).then_some(v);
        }
        if let Some(v) = env_parse("CC_DEFAULT_WAKE_SECONDS") {
            limits.default_wake_seconds = v;
        }
        if let Some(v) = env_parse("CC_MAX_WAKE_TOTAL_SECONDS") {
            limits.max_wake_total_seconds = v;
        }
        if let Some(v) = env_parse("CC_PROACTIVE_POSTS_PER_DAY") {
            limits.proactive_posts_per_day = v;
        }
        if let Some(v) = env_parse("CC_STICKY_WINDOW_SECONDS") {
            limits.sticky_window_seconds = v;
        }
        if let Some(v) = env_parse("CC_DISCUSSION_MAX_ROUNDS") {
            limits.discussion_max_rounds = v;
        }
        let summary = SummaryConfig {
            model: std::env::var("SUMMARY_MODEL").ok().filter(|v| !v.trim().is_empty()),
            ..SummaryConfig::default()
        };
        Ok(ServerConfig {
            bind: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8090".into()),
            db: DbConfig::from_env(None)?,
            db_max_connections: env_parse("DB_MAX_CONNECTIONS").unwrap_or(20),
            web_dist: Some(std::env::var("WEB_DIST").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("web/dist"))),
            cookie_secure: env_flag("COOKIE_SECURE", false),
            limits,
            manage_existing: true,
            search_index_dir: Some(
                std::env::var("SEARCH_INDEX_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("data/search-index")),
            ),
            summary,
            master_key: std::env::var("CREWCONVO_MASTER_KEY").ok().filter(|v| !v.trim().is_empty()),
            egress: EgressPolicy::from_env(),
            sandbox: env_flag("SANDBOX_ENABLED", true),
            public_base_url: std::env::var("PUBLIC_BASE_URL").ok().filter(|v| !v.trim().is_empty()),
        })
    }
}
