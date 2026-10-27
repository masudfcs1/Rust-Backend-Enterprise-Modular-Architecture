use super::AppError;
use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Path, Query, Request},
    http::{StatusCode, request::Parts},
};
use serde::de::DeserializeOwned;
use validator::Validate;

pub struct ValidatedJson<T>(pub T);
impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate + Send,
{
    type Rejection = AppError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) =
            Json::<T>::from_request(req, state)
                .await
                .map_err(|e| match e.status() {
                    StatusCode::PAYLOAD_TOO_LARGE => AppError::PayloadTooLarge,
                    StatusCode::UNSUPPORTED_MEDIA_TYPE => AppError::UnsupportedMediaType,
                    _ => AppError::Validation(
                        "Request body must be valid JSON matching the documented schema".into(),
                    ),
                })?;
        value.validate().map_err(|errors| {
            // Validator's Display may include submitted secrets; return only field names.
            let mut fields: Vec<_> = errors
                .field_errors()
                .keys()
                .map(|s| s.to_string())
                .collect();
            fields.sort();
            AppError::Validation(format!("Invalid fields: {}", fields.join(", ")))
        })?;
        Ok(Self(value))
    }
}

pub struct ApiQuery<T>(pub T);
impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(v)| Self(v))
            .map_err(|_| AppError::Validation("Invalid query parameters".into()))
    }
}

pub struct ApiPath<T>(pub T);
impl<S, T> FromRequestParts<S> for ApiPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(v)| Self(v))
            .map_err(|_| AppError::Validation("Invalid path parameters".into()))
    }
}
