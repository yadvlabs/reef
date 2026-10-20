//! GitHub-shaped error responses.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use reef_core::Error;

pub struct ApiError(pub Error);

pub const DOCS: &str = "https://docs.github.com/rest";

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match &self.0 {
            Error::NotFound => {
                (StatusCode::NOT_FOUND, serde_json::json!({"message": "Not Found", "documentation_url": DOCS, "status": "404"}))
            }
            Error::Unauthorized(m) => {
                (StatusCode::UNAUTHORIZED, serde_json::json!({"message": m, "documentation_url": DOCS, "status": "401"}))
            }
            Error::Forbidden(m) => (StatusCode::FORBIDDEN, serde_json::json!({"message": m, "documentation_url": DOCS, "status": "403"})),
            Error::Validation { message, errors } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::json!({"message": message, "errors": errors, "documentation_url": DOCS, "status": "422"}),
            ),
            Error::Conflict(m) => (StatusCode::CONFLICT, serde_json::json!({"message": m, "documentation_url": DOCS, "status": "409"})),
            Error::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({"message": "Server Error", "documentation_url": DOCS, "status": "500"}),
                )
            }
        };
        // No `WWW-Authenticate: Basic` here: browsers would pop their own credentials dialog
        // on a failed sign-in. Git endpoints send the challenge themselves.
        (status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn not_found() -> ApiError {
    ApiError(Error::NotFound)
}
pub fn unauthorized() -> ApiError {
    ApiError(Error::Unauthorized("Requires authentication".into()))
}
pub fn forbidden(msg: &str) -> ApiError {
    ApiError(Error::Forbidden(msg.into()))
}
