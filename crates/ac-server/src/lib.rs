//! HTTP / WebSocket server: human API, agent protocol (WebSocket, REST, MCP, webhooks),
//! realtime events, static web UI.

pub mod a2a;
pub mod agent_api;
mod api;
mod config;
pub mod embedded;
mod error;
pub mod integrations;
mod launch;
pub mod local;
pub mod longpoll;
pub mod mcp;
mod session;
pub mod webhook;
mod ws;

pub use config::{Edition, ProviderSpec, ServerConfig};
pub use launch::serve_from_env;

/// The release this program was built from: `CREWCONVO_VERSION` at build time (the release
/// workflow passes the tag, e.g. `v0.6.0`). None for a build made without it — a development
/// build, which does not know what it is and therefore never offers updates.
pub fn version() -> Option<&'static str> {
    option_env!("CREWCONVO_VERSION").map(str::trim).filter(|v| !v.is_empty())
}

use std::sync::Arc;

use ac_context::{ContextAssembler, ContextBudget, Summarizer};
use ac_core::bus::Bus;
use ac_decider::{DeciderChain, SecretBox};
use ac_llm::LlmRegistry;
use ac_runtime::BuiltinRuntime;
use ac_sandbox::{DockerSandbox, DockerSandboxConfig, Sandbox};
use ac_search::SearchService;
use ac_service::Service;
use ac_store::Store;
use ac_wake::WakeEngine;
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub service: Service,
    pub runtime: Arc<BuiltinRuntime>,
    pub config: Arc<ServerConfig>,
    pub llm_providers: Arc<Vec<String>>,
    pub summarizer: Summarizer,
    pub webhooks: Arc<webhook::WebhookManager>,
    pub a2a: Arc<a2a::A2aManager>,
    pub polls: longpoll::PollHub,
    /// Coding agents on this computer; None when the server is not running on the user's own computer.
    pub local: Option<local::LocalAgents>,
}

pub struct App {
    pub state: AppState,
    pub router: Router,
}

/// Wires every component together.
pub async fn build(config: ServerConfig, llm: LlmRegistry) -> anyhow::Result<App> {
    build_with_sandbox(config, llm, None).await
}

/// Like [`build`], with an explicit sandbox backend (tests).
pub async fn build_with_sandbox(
    config: ServerConfig,
    llm: LlmRegistry,
    sandbox_override: Option<Arc<dyn Sandbox>>,
) -> anyhow::Result<App> {
    let store = Store::connect(&config.db, config.db_max_connections).await?;
    store.migrate().await?;
    let bus = Bus::new();

    let secrets = match &config.master_key {
        Some(k) => SecretBox::from_base64(k).map_err(anyhow::Error::msg)?,
        None => {
            tracing::warn!(
                "CREWCONVO_MASTER_KEY not set: using an ephemeral key; stored secrets will not survive a restart"
            );
            SecretBox::ephemeral()
        }
    };
    let decider = Arc::new(DeciderChain::new(secrets, config.egress));

    let context = ContextAssembler::new(store.clone(), config.limits.clone(), ContextBudget::default());
    let engine = WakeEngine::start(store.clone(), bus.clone(), config.limits.clone(), context, decider.clone());

    let search = SearchService::open(store.clone(), config.search_index_dir.clone())?;
    search.start_indexer(&bus);

    let sandbox: Option<Arc<dyn Sandbox>> = match sandbox_override {
        Some(s) => Some(s),
        None if config.sandbox => {
            let sb = DockerSandbox::new(DockerSandboxConfig::from_env());
            if sb.available().await {
                tracing::info!(image = %sb.config().image, "docker sandbox available");
                Some(sb as Arc<dyn Sandbox>)
            } else {
                tracing::warn!("docker is not reachable: sandbox tools disabled");
                None
            }
        }
        None => None,
    };

    let service = Service::new(store.clone(), engine.clone(), search.clone(), sandbox, decider);
    let llm_providers = Arc::new(llm.provider_names());
    let summarizer = Summarizer::new(store, llm.clone(), config.summary.clone());
    summarizer.start(&bus);
    let runtime = BuiltinRuntime::new(service.clone(), llm);
    let webhooks = webhook::WebhookManager::new(service.clone());
    let a2a = a2a::A2aManager::new(service.clone());
    if config.manage_existing {
        store_reset_presence(&service).await;
        runtime.load_all().await?;
        webhooks.load_all().await?;
        a2a.load_all().await?;
        {
            // Platform catalogs are re-read every 10 minutes: new agents appear, removed ones go away.
            let service = service.clone();
            let declared = config.providers.clone();
            tokio::spawn(async move {
                let mut first = true;
                loop {
                    // Connections declared in the configuration (A2A_CONNECTIONS) are applied at
                    // startup, and again once the first account exists.
                    if let Some(specs) = &declared
                        && let Err(e) = service.reconcile_providers(specs).await
                    {
                        tracing::warn!(error = %e, "could not apply A2A_CONNECTIONS");
                    }
                    if !first {
                        service.sync_all_providers().await;
                    }
                    first = false;
                    let wait =
                        if declared.is_some() && !service.has_accounts().await.unwrap_or(true) { 15 } else { 600 };
                    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                }
            });
        }
        engine.recover().await?;
        search.ensure_populated();
    }
    let polls = longpoll::PollHub::default();
    {
        // Pollers that stopped polling go offline.
        let polls = polls.clone();
        let service = service.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tick.tick().await;
                for id in polls.reap() {
                    if let Ok(Some(a)) = service.store().get_agent(id).await {
                        let _ = service.agent_connection_changed(&a, false).await;
                    }
                }
            }
        });
    }
    let local = config.local_agents.then(|| {
        let server = local::local_url(&config.bind);
        let opts = local::platforms::RunOptions {
            live: config.local_live_messages,
            codex_windows_sandbox: config.local_codex_windows_sandbox.clone(),
        };
        local::LocalAgents::new(service.clone(), &config.data_dir, server, config.local_sessions_reuse, opts)
    });
    if config.manage_existing
        && let Some(l) = &local
    {
        l.start_all();
    }
    let state =
        AppState { service, runtime, config: Arc::new(config), llm_providers, summarizer, webhooks, a2a, polls, local };
    let router = router(state.clone());
    Ok(App { state, router })
}

fn router(state: AppState) -> Router {
    let mcp = Router::new()
        .nest_service("/mcp", mcp::service(state.clone()))
        .layer(axum::middleware::from_fn_with_state(state.clone(), mcp::auth));
    let mut app = Router::new()
        .merge(api::routes())
        .merge(ws::routes())
        .merge(agent_api::routes())
        .merge(integrations::routes())
        .merge(local::routes())
        .with_state(state.clone())
        .merge(mcp);
    if let Some(dir) = &state.config.web_dist
        && dir.join("index.html").exists()
    {
        let serve = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
        let web = Router::new().fallback_service(serve).layer(axum::middleware::from_fn(web_cache_headers));
        app = app.fallback_service(web);
    }
    app.layer(TraceLayer::new_for_http())
}

/// The page itself is checked with the server on every load, so a new version
/// of the app is never hidden behind a cached page; the files it points to
/// carry a content hash in their name and can be kept.
async fn web_cache_headers(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
    let hashed = req.uri().path().starts_with("/assets/");
    let mut res = next.run(req).await;
    let html = res.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/html"));
    let value = if html {
        "no-cache"
    } else if hashed && res.status().is_success() {
        "public, max-age=31536000, immutable"
    } else {
        return res;
    };
    res.headers_mut().insert(CACHE_CONTROL, axum::http::HeaderValue::from_static(value));
    res
}

async fn store_reset_presence(service: &Service) {
    match service.store().reset_external_presence().await {
        Ok(n) if n > 0 => tracing::info!(count = n, "external agents marked offline until they reconnect"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "could not reset external agent presence"),
    }
}
