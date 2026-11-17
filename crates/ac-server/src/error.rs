use ac_core::AppError;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = match &self.0 {
            AppError::Internal(detail) => {
                tracing::error!(%detail, "internal error");
                "伺服器內部錯誤".to_string()
            }
            AppError::Unauthorized => "請先登入".to_string(),
            AppError::NotFound(m)
            | AppError::Forbidden(m)
            | AppError::BadRequest(m)
            | AppError::Conflict(m)
            | AppError::RateLimited(m) => m.clone(),
        };
        (status, Json(json!({ "error": { "code": self.0.code(), "message": message } }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
