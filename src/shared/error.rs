use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),
    #[error("Authentication required or credentials are invalid")]
    Unauthorized,
    #[error("You do not have permission to perform this action")]
    Forbidden,
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("An internal error occurred")]
    Internal,
    #[error("The service is temporarily unavailable")]
    Unavailable,
    #[error("Too many requests; retry later")]
    TooManyRequests,
    #[error("Request body exceeds the configured limit")]
    PayloadTooLarge,
    #[error("Content-Type must be application/json")]
    UnsupportedMediaType,
    #[error("The request timed out")]
    Timeout,
    #[error("Method not allowed")]
    MethodNotAllowed,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}
#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::Timeout => StatusCode::REQUEST_TIMEOUT,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation_error",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Internal => "internal_error",
            Self::Unavailable => "unavailable",
            Self::TooManyRequests => "rate_limited",
            Self::PayloadTooLarge => "payload_too_large",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::Timeout => "request_timeout",
            Self::MethodNotAllowed => "method_not_allowed",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status(),
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code(),
                    message: self.to_string(),
                },
            }),
        )
            .into_response();
        if matches!(self, Self::Unauthorized) {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        if matches!(self, Self::TooManyRequests) {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, header::HeaderValue::from_static("60"));
        }
        response
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::RowNotFound => Self::NotFound("Resource not found"),
            sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_) => {
                tracing::error!("database connection unavailable");
                Self::Unavailable
            }
            sqlx::Error::Database(ref e) if e.is_unique_violation() => {
                Self::Conflict("Resource already exists")
            }
            sqlx::Error::Database(ref e) => {
                // SQLSTATE contains no query, row values, credentials, or PII.
                tracing::error!(sqlstate = ?e.code(), "database operation failed");
                Self::Internal
            }
            _ => {
                tracing::error!("database operation failed");
                Self::Internal
            }
        }
    }
}
