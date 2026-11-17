//! MCP endpoint (`/mcp`, Streamable HTTP, Bearer token) exposing the agent tools.
//! Shares `Service::call_tool` with REST and the builtin runtime.

use std::sync::Arc;

use ac_core::model::MemberKind;
use ac_service::tools::agent_tool_specs;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::Value;

use crate::AppState;
use crate::agent_api::{AuthedAgentExt, bearer};
use crate::error::ApiError;

#[derive(Clone)]
pub struct McpServer {
    state: AppState,
}

fn authed(ctx: &RequestContext<RoleServer>) -> Result<AuthedAgentExt, McpError> {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|p| p.extensions.get::<AuthedAgentExt>().cloned())
        .ok_or_else(|| McpError::invalid_request("unauthenticated", None))
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("crewconvo", env!("CARGO_PKG_VERSION")))
            // Same guidance as the Skill, for MCP clients without skills.
            .with_instructions(include_str!("../../../integrations/AGENTS.md").replace("{{BASE_URL}}", "this server"))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let a = authed(&context)?;
        let tools = agent_tool_specs(&a.1, self.state.service.sandbox().is_some())
            .into_iter()
            .map(|t| {
                let schema = match t.input_schema {
                    Value::Object(o) => o,
                    _ => serde_json::Map::new(),
                };
                Tool::new(t.name, t.description, Arc::new(schema))
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let a = authed(&context)?;
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let result = match self.state.service.call_tool(&a.0, None, &request.name, args).await {
            Ok(v) => CallToolResult::success(vec![ContentBlock::text(v.to_string())]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(
                serde_json::json!({ "error": e.to_string(), "code": e.code() }).to_string(),
            )]),
        };
        Ok(result.into())
    }
}

/// Bearer-token authentication for `/mcp`.
pub async fn auth(State(s): State<AppState>, mut req: Request, next: Next) -> Response {
    let Some(token) = bearer(req.headers()) else {
        return ApiError(ac_core::AppError::Unauthorized).into_response();
    };
    match s.service.authenticate_agent(&token).await {
        Ok((m, a)) if m.kind == MemberKind::Agent => {
            req.extensions_mut().insert(AuthedAgentExt(m, a));
            next.run(req).await
        }
        Ok(_) => ApiError(ac_core::AppError::Unauthorized).into_response(),
        Err(e) => ApiError(e).into_response(),
    }
}

pub fn service(state: AppState) -> StreamableHttpService<McpServer, LocalSessionManager> {
    let server = McpServer { state };
    // Authentication is by bearer token, so DNS-rebinding host checks are not needed here.
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
    StreamableHttpService::new(move || Ok(server.clone()), Arc::new(LocalSessionManager::default()), config)
}
