use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// `code` is stable and translated by the web UI; internal details are only logged.
pub enum Error {
    Invalid(&'static str),
    Unauthorized,
    BadCredentials,
    Forbidden,
    NotFound,
    Conflict(&'static str),
    TooManyAttempts,
    Internal(String),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Error::Invalid(code) => (StatusCode::UNPROCESSABLE_ENTITY, code),
            Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Error::BadCredentials => (StatusCode::UNAUTHORIZED, "bad_credentials"),
            Error::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Error::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Error::Conflict(code) => (StatusCode::CONFLICT, code),
            Error::TooManyAttempts => (StatusCode::TOO_MANY_REQUESTS, "too_many_attempts"),
            Error::Internal(detail) => {
                tracing::error!("{detail}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        };
        (status, Json(serde_json::json!({ "error": code }))).into_response()
    }
}

impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Error::Internal(error.to_string())
    }
}
