//! Shared test fixture. Tests run against the real PostgreSQL test database
//! (`PG_*` from the repository `.env`, database `crewconvo_test` unless `TEST_PG_DBNAME` is set);
//! every test works in its own fresh workspace.
#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use ac_context::SummaryConfig;
use ac_core::limits::Limits;
use ac_core::model::{Agent, Conversation, Member, Visibility};
use ac_decider::EgressPolicy;
use ac_llm::{LlmConfig, LlmRegistry};
use ac_sandbox::Sandbox;
use ac_server::{App, ServerConfig};
use ac_service::agents::CreateAgentInput;
use ac_store::DbConfig;
use uuid::Uuid;

pub struct Fx {
    pub app: App,
    pub human: Member,
    pub workspace_id: Uuid,
}

#[derive(Default)]
pub struct Opts {
    pub summary_model: Option<String>,
    pub sandbox: Option<Arc<dyn Sandbox>>,
    pub limits: Option<Limits>,
    /// Use real provider keys from the environment (otherwise mock only).
    pub real_llm: bool,
}

pub fn load_env() {
    let _ = dotenvy::from_filename("../../.env");
    let _ =
        tracing_subscriber::fmt().with_env_filter("warn,ac_wake=info,ac_decider=info").with_test_writer().try_init();
}

pub async fn fixture() -> Fx {
    fixture_with(Opts::default()).await
}

pub async fn fixture_with(o: Opts) -> Fx {
    load_env();
    let dbname = std::env::var("TEST_PG_DBNAME").unwrap_or_else(|_| "crewconvo_test".into());
    let config = ServerConfig {
        edition: ac_server::Edition::Enterprise,
        registration_open: true,
        providers: None,
        bind: "127.0.0.1:0".into(),
        db: DbConfig::from_env(Some(&dbname)).expect("db config"),
        db_max_connections: 5,
        web_dist: None,
        cookie_secure: false,
        limits: o.limits.unwrap_or_default(),
        manage_existing: false,
        search_index_dir: None,
        summary: SummaryConfig { model: o.summary_model, ..SummaryConfig::default() },
        master_key: None,
        // Tests run fake deciders / webhooks on localhost over http.
        egress: EgressPolicy { allow_private: true, allow_http: true },
        sandbox: false,
        public_base_url: None,
        data_dir: std::env::temp_dir(),
        local_agents: false,
    };
    let llm_cfg = if o.real_llm {
        LlmConfig { enable_mock: true, ..LlmConfig::from_env() }
    } else {
        LlmConfig { enable_mock: true, ..Default::default() }
    };
    let llm = LlmRegistry::from_config(&llm_cfg);
    let app = ac_server::build_with_sandbox(config, llm, o.sandbox).await.expect("build app");
    let store = app.state.service.store().clone();
    let ws = store.create_workspace(&format!("test-{}", Uuid::now_v7())).await.unwrap();
    let human = store.create_human(ws.id, "alice", "Alice", "x", true).await.unwrap();
    Fx { app, human, workspace_id: ws.id }
}

impl Fx {
    pub fn svc(&self) -> &ac_service::Service {
        &self.app.state.service
    }
    pub fn store(&self) -> &ac_store::Store {
        self.svc().store()
    }

    pub async fn agent(&self, handle: &str, model: &str) -> (Agent, Member) {
        self.agent_with(handle, model, &format!("測試用 Agent {handle}"), vec![]).await
    }

    pub async fn agent_with(
        &self,
        handle: &str,
        model: &str,
        description: &str,
        expertise: Vec<&str>,
    ) -> (Agent, Member) {
        let a = self
            .svc()
            .create_builtin_agent(
                &self.human,
                CreateAgentInput {
                    handle: handle.into(),
                    display_name: None,
                    description: description.into(),
                    expertise: expertise.into_iter().map(String::from).collect(),
                    model: model.into(),
                    system_prompt: None,
                    tools: vec![],
                    max_wake_seconds: Some(120),
                    daily_token_budget: None,
                    sandbox_config: None,
                    join_conversations: vec![],
                },
            )
            .await
            .expect("create agent");
        let m = self.store().get_member(a.member.id).await.unwrap().unwrap();
        (a, m)
    }

    pub async fn channel(&self, invite: &[Uuid]) -> Conversation {
        self.svc()
            .create_channel(&self.human, &format!("test-{}", Uuid::now_v7().simple()), None, Visibility::Public, invite)
            .await
            .expect("create channel")
    }

    pub async fn second_human(&self, handle: &str, admin: bool) -> Member {
        self.store().create_human(self.workspace_id, handle, handle, "x", admin).await.unwrap()
    }

    /// Serves the router on an ephemeral local port; returns the base URL.
    pub async fn serve(&self) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let router = self.app.router.clone();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }
}

pub async fn wait_for<F, Fut>(what: &str, timeout: Duration, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if f().await {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for: {what}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Engine input drained and no pending / in-flight wakes in the conversation.
pub async fn settled(fx: &Fx, conv: Uuid) -> bool {
    fx.svc().engine().is_input_idle() && fx.store().active_wakes_for_conversation(conv).await.unwrap().is_empty()
}
