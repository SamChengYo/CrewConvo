//! Endpoints for other platforms: the `crewconvo connect` browser-approval flow and the
//! Skill / agent instructions (templated with this server's public URL).
//!
//! The CLI itself is installed from the repository (`clients/python`), not from this server.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::error::ApiResult;

const SKILL: &str = include_str!("../../../clients/python/crewconvo/SKILL.md");
const AGENTS: &str = include_str!("../../../integrations/AGENTS.md");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/integrations/SKILL.md", get(|s, h| file(s, h, SKILL)))
        .route("/integrations/AGENTS.md", get(|s, h| file(s, h, AGENTS)))
        .route("/agent/v1/connect/start", post(connect_start))
        .route("/agent/v1/connect/poll", post(connect_poll))
}

/// The URL other machines should use: `PUBLIC_BASE_URL`, else derived from the request.
pub fn base_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(u) = &state.config.public_base_url {
        return u.trim_end_matches('/').to_string();
    }
    let h = |k: &str| {
        headers.get(k).and_then(|v| v.to_str().ok()).map(|s| s.split(',').next().unwrap_or("").trim().to_string())
    };
    let proto = h("x-forwarded-proto").filter(|p| p == "https" || p == "http").unwrap_or_else(|| "http".into());
    let host = h("x-forwarded-host").or_else(|| h("host")).unwrap_or_else(|| "localhost:8090".into());
    format!("{proto}://{host}")
}

async fn file(State(s): State<AppState>, headers: HeaderMap, body: &'static str) -> impl IntoResponse {
    let text = body.replace("{{BASE_URL}}", &base_url(&s, &headers));
    ([(CONTENT_TYPE, "text/markdown; charset=utf-8")], text)
}

#[derive(Deserialize)]
struct StartBody {
    #[serde(default)]
    client: Value,
}

async fn connect_start(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<StartBody>,
) -> ApiResult<impl IntoResponse> {
    let r = s.service.connect_start(b.client).await?;
    let base = base_url(&s, &headers);
    Ok(Json(json!({
        "device_code": r.device_code,
        "user_code": r.user_code,
        "expires_in": r.expires_in,
        "interval": r.interval,
        "verification_url": format!("{base}/connect"),
        "verification_url_complete": format!("{base}/connect?code={}", r.user_code),
    })))
}

#[derive(Deserialize)]
struct PollBody {
    device_code: String,
}

async fn connect_poll(State(s): State<AppState>, Json(b): Json<PollBody>) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.connect_poll(&b.device_code).await?))
}
