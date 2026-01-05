use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(thiserror::Error, Debug)]
pub enum ApiError {
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("User not found")]
    UserNotFound,
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Resource not found: {0}")]
    NotFound(String),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Internal server error: {0}")]
    InternalError(#[from] anyhow::Error),
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::AuthenticationFailed(msg) => (StatusCode::UNAUTHORIZED, msg),
            ApiError::UserNotFound => (StatusCode::UNAUTHORIZED, "User not found".to_string()),
            ApiError::PermissionDenied(msg) => (StatusCode::FORBIDDEN, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::InvalidRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            ApiError::InternalError(err) => {
                tracing::error!("Internal server error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            ApiError::DatabaseError(err) => {
                tracing::error!("Database error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error".to_string(),
                )
            }
        };

        let body = Json(json!({
            "error": message
        }));

        (status, body).into_response()
    }
}
