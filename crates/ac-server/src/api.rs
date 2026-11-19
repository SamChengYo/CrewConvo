//! Human REST API (spec 10).

use ac_core::AppError;
use ac_core::model::{Member, Visibility};
use ac_service::agents::{CreateAgentInput, UpdateAgentInput};
use ac_service::auth::SESSION_DAYS;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::http::header::SET_COOKIE;
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiResult;
use crate::session::{CurrentMember, SESSION_COOKIE, cookie_value, session_cookie};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(|| async { Json(json!({ "ok": true })) }))
        .route("/api/auth/info", get(auth_info))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/members", get(members))
        .route("/api/models", get(models))
        .route("/api/agents", get(list_agents).post(create_agent))
        .route("/api/agents/{id}", get(get_agent).patch(update_agent).delete(delete_agent))
        .route("/api/conversations", get(list_conversations).post(create_channel))
        .route("/api/channels/public", get(public_channels))
        .route("/api/dms", post(open_dm))
        .route("/api/conversations/{id}", get(conversation_detail))
        .route("/api/conversations/{id}/join", post(join))
        .route("/api/conversations/{id}/leave", post(leave))
        .route("/api/conversations/{id}/invite", post(invite))
        .route("/api/conversations/{id}/stop", post(stop))
        .route("/api/conversations/{id}/read", post(mark_read))
        .route("/api/conversations/{id}/messages", get(history).post(post_message))
        .route("/api/messages/{id}", delete(delete_message))
        .route("/api/messages/{id}/reactions", post(add_reaction).delete(remove_reaction))
        .route("/api/messages/{id}/wakes", get(explain_wakes))
        .route("/api/search", get(search))
        .route("/api/activity", get(activity))
        .route("/api/provider-connections", get(list_providers).post(connect_provider))
        .route("/api/provider-connections/{id}", delete(delete_provider))
        .route("/api/provider-connections/{id}/sync", post(sync_provider))
        .route("/api/agent-groups", get(list_groups).post(create_group))
        .route("/api/agent-groups/{id}", patch(update_group).delete(delete_group))
        .route("/api/connect/{code}", get(connect_request))
        .route("/api/connect/{code}/approve", post(connect_approve))
        .route("/api/connect/{code}/deny", post(connect_deny))
        .route("/api/wakes/{id}/payload", get(wake_payload))
        .route("/api/system", get(system))
        .route("/api/decider-configs", get(list_deciders).post(create_decider))
        .route("/api/decider-configs/reorder", post(reorder_deciders))
        .route("/api/decider-configs/{id}", axum::routing::patch(update_decider).delete(delete_decider))
        .route("/api/decider-configs/{id}/test", post(test_decider))
        .route("/api/agents/external", post(create_external_agent))
        .route("/api/agents/{id}/token", post(rotate_token).delete(revoke_token))
        .route("/api/agents/{id}/delivery", axum::routing::put(update_delivery))
        .route("/api/agents/{id}/sandbox", axum::routing::put(update_sandbox))
}

#[derive(Deserialize)]
struct RegisterBody {
    /// E-mail + name + password is the sign-up form; `handle` is still accepted from older clients.
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    display_name: String,
    password: String,
}

async fn register(State(s): State<AppState>, Json(b): Json<RegisterBody>) -> ApiResult<impl IntoResponse> {
    // The first account can always be created; after that it depends on the edition and REGISTRATION.
    if !s.config.registration_open && s.service.has_accounts().await? {
        let why = if s.config.edition == crate::Edition::Personal {
            "Personal 版只有一個帳號，已經建立過了"
        } else {
            "這個站台沒有開放註冊，請聯絡管理員"
        };
        return Err(crate::error::ApiError(ac_core::AppError::forbidden(why)));
    }
    let member = match (&b.email, &b.name, &b.handle) {
        (Some(email), Some(name), _) => s.service.register_account(email, name, &b.password).await?,
        (_, _, Some(handle)) => s.service.register(handle, &b.display_name, &b.password).await?,
        _ => return Err(crate::error::ApiError(ac_core::AppError::bad_request("請填寫信箱、名字與密碼"))),
    };
    login_response(&s, &member.handle, &b.password).await
}

#[derive(Deserialize)]
struct LoginBody {
    handle: String,
    password: String,
}

async fn login(State(s): State<AppState>, Json(b): Json<LoginBody>) -> ApiResult<impl IntoResponse> {
    login_response(&s, &b.handle, &b.password).await
}

async fn login_response(s: &AppState, handle: &str, password: &str) -> ApiResult<(HeaderMap, Json<Member>)> {
    let (member, token) = s.service.login(handle, password).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        SET_COOKIE,
        session_cookie(&token, SESSION_DAYS * 86400, s.config.cookie_secure).parse().expect("cookie header"),
    );
    Ok((headers, Json(member)))
}

async fn logout(State(s): State<AppState>, req: axum::extract::Request) -> ApiResult<impl IntoResponse> {
    let (parts, _) = req.into_parts();
    if let Some(token) = cookie_value(&parts, SESSION_COOKIE) {
        s.service.logout(&token).await?;
    }
    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, session_cookie("", 0, s.config.cookie_secure).parse().expect("cookie header"));
    Ok((headers, Json(json!({ "ok": true }))))
}

async fn me(CurrentMember(m): CurrentMember) -> Json<Member> {
    Json(m)
}

async fn members(State(s): State<AppState>, CurrentMember(m): CurrentMember) -> ApiResult<Json<Vec<Member>>> {
    Ok(Json(s.service.store().list_members(m.workspace_id).await?))
}

async fn models(State(s): State<AppState>, CurrentMember(_): CurrentMember) -> Json<Value> {
    Json(json!({
        "providers": *s.llm_providers,
        "examples": ["anthropic:claude-sonnet-5-5", "openai:gpt-6-luna", "gemini:gemini-4-hydrogen", "anthropic:claude-haiku-5-5", "mock:reply", "mock:react", "mock:silent"],
    }))
}

async fn list_agents(State(s): State<AppState>, CurrentMember(m): CurrentMember) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.list_agent_records(&m).await?))
}

async fn create_agent(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Json(b): Json<CreateAgentInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.create_builtin_agent(&m, b).await?))
}

async fn get_agent(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.get_agent(&m, id).await?))
}

async fn update_agent(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<UpdateAgentInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.update_agent(&m, id, b).await?))
}

async fn list_conversations(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
) -> ApiResult<impl IntoResponse> {
    let items = s.service.list_conversations(&m).await?;
    // DMs are displayed by their members.
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let members = if it.conversation.kind == ac_core::model::ConversationKind::Dm {
            s.service.store().conversation_members(it.conversation.id).await?
        } else {
            vec![]
        };
        let mut v = serde_json::to_value(&it).map_err(AppError::internal)?;
        v["dm_members"] = serde_json::to_value(&members).map_err(AppError::internal)?;
        out.push(v);
    }
    Ok(Json(out))
}

async fn public_channels(State(s): State<AppState>, CurrentMember(m): CurrentMember) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.list_public_channels(&m).await?))
}

#[derive(Deserialize)]
struct CreateChannelBody {
    name: String,
    topic: Option<String>,
    #[serde(default = "default_visibility")]
    visibility: Visibility,
    #[serde(default)]
    invite: Vec<Uuid>,
}

fn default_visibility() -> Visibility {
    Visibility::Public
}

async fn create_channel(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Json(b): Json<CreateChannelBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.create_channel(&m, &b.name, b.topic.as_deref(), b.visibility, &b.invite).await?))
}

#[derive(Deserialize)]
struct MemberIdsBody {
    member_ids: Vec<Uuid>,
}

async fn open_dm(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Json(b): Json<MemberIdsBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.open_dm(&m, &b.member_ids).await?))
}

async fn conversation_detail(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.conversation_detail(&m, id).await?))
}

async fn join(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    s.service.join(&m, id).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn leave(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    s.service.leave(&m, id).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn invite(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<MemberIdsBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(json!({ "added": s.service.invite(&m, id, &b.member_ids).await? })))
}

async fn stop(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let cancelled = s.service.stop_conversation(&m, id).await?;
    Ok(Json(json!({ "cancelled": cancelled.len() })))
}

#[derive(Deserialize)]
struct ReadBody {
    seq: i64,
}

async fn mark_read(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<ReadBody>,
) -> ApiResult<impl IntoResponse> {
    s.service.mark_read(&m, id, b.seq).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct HistoryQuery {
    thread_root_id: Option<Uuid>,
    before_seq: Option<i64>,
    limit: Option<i64>,
}

async fn history(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.read_history(&m, id, q.thread_root_id, q.before_seq, q.limit).await?))
}

#[derive(Deserialize)]
struct PostBody {
    body: String,
    thread_root_id: Option<Uuid>,
}

async fn post_message(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<PostBody>,
) -> ApiResult<impl IntoResponse> {
    // Without a decision model nobody decides who answers an untagged message, so the app is
    // not usable yet; the web UI shows the setup screen, and this keeps other clients honest.
    if s.config.require_decider && !s.service.decider_ready(m.workspace_id).await? {
        return Err(AppError::Conflict("請先設定決策模型".into()).into());
    }
    Ok(Json(s.service.post_message(&m, None, id, &b.body, b.thread_root_id).await?))
}

async fn delete_message(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    s.service.delete_message(&m, id).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct EmojiBody {
    emoji: String,
}

async fn add_reaction(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<EmojiBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.react(&m, id, &b.emoji, true).await?))
}

async fn remove_reaction(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<EmojiBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.react(&m, id, &b.emoji, false).await?))
}

async fn explain_wakes(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.explain_wakes(&m, id).await?))
}

async fn search(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Query(p): Query<ac_search::SearchParams>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.search().search(&m, &p).await?))
}

async fn system(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    // With the requirement turned off the app never asks for a decision model.
    let decider_ready = !s.config.require_decider || s.service.decider_ready(m.workspace_id).await?;
    Ok(Json(json!({
        "decider_ready": decider_ready,
        "version": crate::version(),
        "edition": s.config.edition.as_str(),
        "registration_open": s.config.registration_open,
        "public_base_url": crate::integrations::base_url(&s, &headers),
        "public_base_url_configured": s.config.public_base_url.is_some(),
        "llm_providers": *s.llm_providers,
        "sandbox": s.service.sandbox().is_some(),
        "summaries": s.summarizer.enabled(),
        "summary_model": s.config.summary.model,
        "limits": s.service.limits(),
        "egress": { "allow_private": s.config.egress.allow_private, "allow_http": s.config.egress.allow_http },
        "local_agents": s.local.is_some(),
        "mcp_endpoint": "/mcp",
        "agent_ws_endpoint": "/agent/v1/ws",
    })))
}

async fn list_deciders(State(s): State<AppState>, CurrentMember(m): CurrentMember) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.list_decider_configs(&m).await?))
}

async fn create_decider(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Json(b): Json<ac_service::deciders::DeciderConfigInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.create_decider_config(&m, b).await?))
}

async fn update_decider(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<ac_service::deciders::DeciderConfigInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.update_decider_config(&m, id, b).await?))
}

async fn delete_decider(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    s.service.delete_decider_config(&m, id).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ReorderBody {
    ids: Vec<Uuid>,
}

async fn reorder_deciders(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Json(b): Json<ReorderBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.reorder_decider_configs(&m, &b.ids).await?))
}

async fn test_decider(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.test_decider_config(&m, id).await?))
}

async fn create_external_agent(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Json(b): Json<ac_service::external::CreateExternalAgentInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.create_external_agent(&m, b).await?))
}

async fn rotate_token(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.rotate_agent_token(&m, id).await?))
}

async fn revoke_token(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.revoke_agent_token(&m, id).await?))
}

async fn update_delivery(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<ac_service::external::DeliveryInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.update_agent_delivery(&m, id, b).await?))
}

async fn update_sandbox(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.set_sandbox_limits(&m, id, b).await?))
}

#[derive(Deserialize)]
struct ConfirmQuery {
    #[serde(default)]
    confirm: bool,
}

async fn delete_agent(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Query(q): Query<ConfirmQuery>,
) -> ApiResult<impl IntoResponse> {
    s.service.delete_agent(&m, id, q.confirm).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ActivityQuery {
    before: Option<chrono::DateTime<chrono::Utc>>,
    limit: Option<i64>,
}

async fn activity(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Query(q): Query<ActivityQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.activity(&m, q.before, q.limit).await?))
}

async fn wake_payload(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.wake_payload(&m, id).await?))
}

async fn connect_request(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(code): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.connect_request(&m, &code).await?))
}

async fn connect_approve(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(code): Path<String>,
    Json(b): Json<ac_service::connect::ApproveInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.connect_approve(&m, &code, b).await?))
}

async fn connect_deny(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(code): Path<String>,
) -> ApiResult<impl IntoResponse> {
    s.service.connect_deny(&m, &code).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn list_groups(State(s): State<AppState>, CurrentMember(m): CurrentMember) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.list_agent_groups(&m).await?))
}

async fn create_group(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Json(b): Json<ac_service::groups::GroupInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.create_agent_group(&m, b).await?))
}

async fn update_group(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
    Json(b): Json<ac_service::groups::GroupInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.update_agent_group(&m, id, b).await?))
}

async fn delete_group(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    s.service.delete_agent_group(&m, id).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn list_providers(State(s): State<AppState>, CurrentMember(m): CurrentMember) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.list_provider_connections(&m).await?))
}

async fn connect_provider(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Json(b): Json<ac_service::provider::ConnectProviderInput>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.connect_provider(&m, b).await?))
}

async fn sync_provider(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(s.service.sync_provider_connection(&m, id).await?))
}

async fn delete_provider(
    State(s): State<AppState>,
    CurrentMember(m): CurrentMember,
    Path(id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    s.service.delete_provider_connection(&m, id).await?;
    Ok(Json(json!({ "ok": true })))
}

/// Public: what the sign-in page needs to know.
async fn auth_info(State(s): State<AppState>) -> ApiResult<impl IntoResponse> {
    let has_accounts = s.service.has_accounts().await?;
    Ok(Json(json!({
        "edition": s.config.edition.as_str(),
        "needs_setup": !has_accounts,
        "registration_open": s.config.registration_open || !has_accounts,
    })))
}
