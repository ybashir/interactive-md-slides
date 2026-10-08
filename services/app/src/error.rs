use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication required")]
    Unauthorized,
    #[error("you do not have access to this resource")]
    Forbidden,
    #[error("resource not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    ResponseIntegrity(String),
    #[error("Too many requests; try again in {0} seconds")]
    RateLimited(u64),
    #[error("{0}")]
    Upstream(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    External(#[from] reqwest::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let retry_after = match &self {
            Self::RateLimited(seconds) => Some(*seconds),
            _ => None,
        };
        let (status, code, message) = match &self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", self.to_string()),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", self.to_string()),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request", self.to_string()),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict", self.to_string()),
            Self::ResponseIntegrity(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "response_integrity",
                self.to_string(),
            ),
            Self::RateLimited(_) => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                self.to_string(),
            ),
            Self::Upstream(_) => (StatusCode::BAD_GATEWAY, "upstream_error", self.to_string()),
            Self::Database(error) => {
                tracing::error!(?error, "database request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database_error",
                    "The database request failed".to_owned(),
                )
            }
            Self::External(error) => {
                tracing::error!(?error, "external request failed");
                (
                    StatusCode::BAD_GATEWAY,
                    "external_error",
                    "An external service request failed".to_owned(),
                )
            }
            Self::Internal(error) => {
                tracing::error!(?error, "internal request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "An internal error occurred".to_owned(),
                )
            }
        };

        let mut response = (
            status,
            Json(json!({ "error": {
                "code": code,
                "message": message,
                "retry_after_seconds": retry_after,
            } })),
        )
            .into_response();
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

pub type AppResult<T> = Result<T, AppError>;
