use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::NotFound(_) => "not_found",
            AppError::Unauthorized => "unauthorized",
            AppError::Forbidden(_) => "forbidden",
            AppError::BadRequest(_) => "bad_request",
            AppError::Conflict(_) => "conflict",
            AppError::RateLimited(_) => "rate_limited",
            AppError::Internal(_) => "internal",
        }
    }

    pub fn not_found(what: impl Into<String>) -> Self {
        AppError::NotFound(what.into())
    }
    pub fn forbidden(what: impl Into<String>) -> Self {
        AppError::Forbidden(what.into())
    }
    pub fn bad_request(what: impl Into<String>) -> Self {
        AppError::BadRequest(what.into())
    }
    pub fn internal(what: impl std::fmt::Display) -> Self {
        AppError::Internal(what.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
