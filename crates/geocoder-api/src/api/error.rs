use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

/// Error body, like the BAN API: `{"code": 400, "message": "…"}`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: u16,
    pub message: String,
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    /// The country is configured but its index is not built yet.
    Unavailable(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, m),
        };
        (
            status,
            Json(ErrorBody {
                code: status.as_u16(),
                message,
            }),
        )
            .into_response()
    }
}
