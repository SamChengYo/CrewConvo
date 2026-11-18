use std::path::PathBuf;

use ac_context::SummaryConfig;
use ac_core::limits::Limits;
use ac_decider::EgressPolicy;
use ac_store::DbConfig;

/// Personal: one person, built-in database, local by default. Enterprise: many people, external
/// PostgreSQL, registration and admin features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    Personal,
    Enterprise,
}

impl Edition {
    /// `CREWCONVO_EDITION=personal|enterprise` (default: personal).
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("CREWCONVO_EDITION").unwrap_or_default().trim().to_ascii_lowercase().as_str() {
            "" | "personal" => Ok(Edition::Personal),
            "enterprise" => Ok(Edition::Enterprise),
            other => Err(format!("CREWCONVO_EDITION must be personal or enterprise (got {other})")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Edition::Personal => "personal",
            Edition::Enterprise => "enterprise",
        }
    }
}

/// A platform connection declared in the configuration (`A2A_CONNECTIONS`).
pub type ProviderSpec = ac_service::provider::ConnectProviderInput;

/// `A2A_CONNECTIONS` is a JSON array: `[{"url": "https://…/catalog", "token": "…", "name": "…"}]`.
/// Unset → connections are left as they are; set (even to `[]`) → the list is authoritative.
pub fn providers_from_env() -> Result<Option<Vec<ProviderSpec>>, String> {
    match std::env::var("A2A_CONNECTIONS") {
        Ok(v) if !v.trim().is_empty() => serde_json::from_str::<Vec<ProviderSpec>>(v.trim())
            .map(Some)
            .map_err(|e| format!("A2A_CONNECTIONS is not a valid JSON array: {e}")),
        _ => Ok(None),
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub edition: Edition,
    /// Whether new people can create an account (the first account can always be created).
    pub registration_open: bool,
    /// Platform connections from the configuration; None → not managed by configuration.
    pub providers: Option<Vec<ProviderSpec>>,
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
    /// Where the built-in database, search index and generated keys live.
    pub data_dir: PathBuf,
    /// Offer to set up the coding agents on the computer the server runs on (`LOCAL_AGENTS`).
    /// Default: on for the Personal edition when only this computer can reach the server.
    pub local_agents: bool,
    /// A wake of a local coding agent continues a fitting earlier session of that tool instead
    /// of always starting a new one (`LOCAL_SESSIONS=reuse|new`, default reuse).
    pub local_sessions_reuse: bool,
    /// People cannot post until the workspace has a usable decision model (`REQUIRE_DECIDER`,
    /// default on). Off in tests, which mostly run without one.
    pub require_decider: bool,
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

fn env_flag(key: &str, default: bool) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

/// `web/dist` (source checkout) or a `web` folder next to the executable (release archive).
fn web_dist_from_env() -> PathBuf {
    if let Ok(v) = std::env::var("WEB_DIST")
        && !v.trim().is_empty()
    {
        return PathBuf::from(v);
    }
    let local = PathBuf::from("web/dist");
    if local.join("index.html").exists() {
        return local;
    }
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("web")));
    match beside {
        Some(b) if b.join("index.html").exists() => b,
        _ => local,
    }
}

impl ServerConfig {
    /// Where the built-in database, search index and generated keys live (`DATA_DIR`, default `data`).
    pub fn data_dir_from_env() -> PathBuf {
        std::env::var("DATA_DIR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| "data".into())
    }

    pub fn from_env(db: DbConfig) -> Result<Self, String> {
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
        let edition = Edition::from_env()?;
        let data_dir = Self::data_dir_from_env();
        // Personal: only this computer by default. Enterprise: reachable from the network.
        let default_bind = if edition == Edition::Personal { "127.0.0.1:8090" } else { "0.0.0.0:8090" };
        let bind = std::env::var("BIND_ADDR").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default_bind.into());
        let loopback = ["127.", "localhost:", "[::1]:"].iter().any(|p| bind.starts_with(p));
        let local_agents = env_flag("LOCAL_AGENTS", edition == Edition::Personal && loopback);
        let registration_open = match std::env::var("REGISTRATION").ok().map(|v| v.trim().to_ascii_lowercase()) {
            None => edition == Edition::Enterprise,
            Some(v) if v.is_empty() => edition == Edition::Enterprise,
            Some(v) => match v.as_str() {
                "open" => true,
                "closed" => false,
                other => return Err(format!("REGISTRATION must be open or closed (got {other})")),
            },
        };
        let local_sessions_reuse =
            match std::env::var("LOCAL_SESSIONS").unwrap_or_default().trim().to_ascii_lowercase().as_str() {
                "" | "reuse" => true,
                "new" => false,
                other => return Err(format!("LOCAL_SESSIONS must be reuse or new (got {other})")),
            };
        Ok(ServerConfig {
            edition,
            // The Personal edition has exactly one human account.
            registration_open: registration_open && edition == Edition::Enterprise,
            providers: providers_from_env()?,
            bind,
            db,
            db_max_connections: env_parse("DB_MAX_CONNECTIONS").unwrap_or(20),
            web_dist: Some(web_dist_from_env()),
            cookie_secure: env_flag("COOKIE_SECURE", false),
            limits,
            manage_existing: true,
            search_index_dir: Some(
                std::env::var("SEARCH_INDEX_DIR").map(PathBuf::from).unwrap_or_else(|_| data_dir.join("search-index")),
            ),
            summary,
            master_key: std::env::var("CREWCONVO_MASTER_KEY").ok().filter(|v| !v.trim().is_empty()),
            egress: EgressPolicy::from_env(),
            sandbox: env_flag("SANDBOX_ENABLED", true),
            public_base_url: std::env::var("PUBLIC_BASE_URL").ok().filter(|v| !v.trim().is_empty()),
            data_dir,
            local_agents,
            local_sessions_reuse,
            require_decider: env_flag("REQUIRE_DECIDER", true),
        })
    }
}
