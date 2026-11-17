//! Agent protocol endpoints (spec 5): WebSocket events, REST actions, replies.
//! Every action goes through `Service::call_tool` — the same path the builtin runtime uses.

use std::time::Duration;

use ac_core::AppError;
use ac_core::model::{Agent, Member};
use ac_core::protocol::{AgentEvent, AgentReply};
use ac_service::tools::agent_tool_specs;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Path, State};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/agent/v1/ws", get(ws_upgrade))
        .route("/agent/v1/me", get(me))
        .route("/agent/v1/tools", get(list_tools))
        .route("/agent/v1/tools/{name}", post(call_tool))
        .route("/agent/v1/replies", post(reply))
        .route("/agent/v1/events", get(poll_events))
}

/// An agent authenticated by `Authorization: Bearer <token>`.
pub struct AuthedAgent(pub Member, pub Agent);

pub fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(|s| s.trim().to_string())
}

impl FromRequestParts<AppState> for AuthedAgent {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(a) = parts.extensions.get::<AuthedAgentExt>() {
            return Ok(AuthedAgent(a.0.clone(), a.1.clone()));
        }
        let token = bearer(&parts.headers).ok_or(ApiError(AppError::Unauthorized))?;
        let (m, a) = state.service.authenticate_agent(&token).await?;
        Ok(AuthedAgent(m, a))
    }
}

/// Request extension set by the MCP auth middleware.
#[derive(Clone)]
pub struct AuthedAgentExt(pub Member, pub Agent);

fn wake_header(headers: &HeaderMap) -> Result<Option<Uuid>, ApiError> {
    match headers.get("x-wake-id").and_then(|v| v.to_str().ok()) {
        Some(s) => s.trim().parse().map(Some).map_err(|_| ApiError(AppError::bad_request("X-Wake-Id 不是有效的 ID"))),
        None => Ok(None),
    }
}

async fn me(AuthedAgent(_, a): AuthedAgent) -> Json<Agent> {
    Json(a)
}

async fn list_tools(State(s): State<AppState>, AuthedAgent(_, a): AuthedAgent) -> Json<Value> {
    Json(json!({ "tools": agent_tool_specs(&a, s.service.sandbox().is_some()) }))
}

async fn call_tool(
    State(s): State<AppState>,
    AuthedAgent(m, _): AuthedAgent,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> ApiResult<impl IntoResponse> {
    let wake = wake_header(&headers)?;
    let args = body.map(|b| b.0).unwrap_or(Value::Null);
    let result = s.service.call_tool(&m, wake, &name, args).await?;
    Ok(Json(json!({ "ok": true, "result": result })))
}

/// `wake.ack` / `wake.heartbeat` / `wake.done` over HTTP (webhook agents, or WS clients that prefer it).
async fn reply(
    State(s): State<AppState>,
    AuthedAgent(m, _): AuthedAgent,
    Json(r): Json<AgentReply>,
) -> ApiResult<impl IntoResponse> {
    s.service.engine().replier_for(m.id).reply(r).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn ws_upgrade(
    State(s): State<AppState>,
    AuthedAgent(m, a): AuthedAgent,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run_ws(s, m, a, socket))
}

async fn run_ws(s: AppState, member: Member, agent: Agent, mut socket: WebSocket) {
    let mut link = s.service.engine().connect(member.id);
    let _ = s.service.agent_connection_changed(&agent, true).await;
    tracing::info!(agent = %member.handle, "external agent connected (websocket)");
    let hello = json!({ "type": "hello", "agent": { "id": member.id, "handle": member.handle }, "protocol": "crewconvo.agent.v1" });
    let _ = socket.send(WsMessage::Text(hello.to_string().into())).await;
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await;
    loop {
        tokio::select! {
            ev = link.events.recv() => match ev {
                Some(ev) => {
                    let Ok(text) = serde_json::to_string(&ev) else { continue };
                    if socket.send(WsMessage::Text(text.into())).await.is_err() { break; }
                }
                // Connection replaced (another socket) or revoked.
                None => break,
            },
            _ = ping.tick() => {
                let text = serde_json::to_string(&AgentEvent::Ping).unwrap_or_default();
                if socket.send(WsMessage::Text(text.into())).await.is_err() { break; }
            }
            msg = socket.recv() => match msg {
                Some(Ok(WsMessage::Text(t))) => {
                    match serde_json::from_str::<AgentReply>(t.as_str()) {
                        Ok(r) => {
                            if let Err(e) = link.replier.reply(r).await {
                                let err = json!({"type":"error","message": e.to_string()});
                                let _ = socket.send(WsMessage::Text(err.to_string().into())).await;
                            }
                        }
                        Err(e) => {
                            let err = json!({"type":"error","message": format!("無法解析訊息：{e}")});
                            let _ = socket.send(WsMessage::Text(err.to_string().into())).await;
                        }
                    }
                }
                Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
    link.replier.disconnect();
    let still_connected = s.service.engine().is_connected(member.id);
    if !still_connected {
        let _ = s.service.agent_connection_changed(&agent, false).await;
    }
    tracing::info!(agent = %member.handle, "external agent disconnected");
}

#[derive(serde::Deserialize)]
struct PollQuery {
    timeout: Option<u64>,
}

/// Long-poll: blocks up to `timeout` seconds (default 25, max 60) for the next events.
async fn poll_events(
    State(s): State<AppState>,
    AuthedAgent(m, a): AuthedAgent,
    axum::extract::Query(q): axum::extract::Query<PollQuery>,
) -> ApiResult<impl IntoResponse> {
    let (link, fresh) = s.polls.link(s.service.engine(), m.id);
    if fresh {
        s.service.agent_connection_changed(&a, true).await?;
        tracing::info!(agent = %m.handle, "external agent connected (long-poll)");
    }
    let timeout = Duration::from_secs(q.timeout.unwrap_or(25).clamp(1, 60));
    let events = crate::longpoll::PollHub::wait(&link, timeout).await;
    s.polls.touch(m.id);
    match events {
        Some(events) => Ok(Json(json!({ "events": events }))),
        None => Err(ApiError(AppError::Conflict("此 Agent 的連線已被另一個連線取代或撤銷，請重新輪詢".into()))),
    }
}
