use ac_core::AppError;
use ac_core::model::Member;
use axum::extract::FromRequestParts;
use axum::http::header::COOKIE;
use axum::http::request::Parts;

use crate::AppState;
use crate::error::ApiError;

pub const SESSION_COOKIE: &str = "cc_session";

pub fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    parts
        .headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

pub fn session_cookie(token: &str, max_age_secs: i64, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{}",
        if secure { "; Secure" } else { "" }
    )
}

/// The logged-in human.
pub struct CurrentMember(pub Member);

impl FromRequestParts<AppState> for CurrentMember {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = cookie_value(parts, SESSION_COOKIE).ok_or(ApiError(AppError::Unauthorized))?;
        let member = state.service.session_member(&token).await?.ok_or(ApiError(AppError::Unauthorized))?;
        Ok(CurrentMember(member))
    }
}
