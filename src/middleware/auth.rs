use crate::{bootstrap::AppState, modules::auth::Principal, shared::AppError};
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};

pub struct AuthenticatedUser(pub Principal);

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = AppError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let mut headers = parts.headers.get_all(header::AUTHORIZATION).iter();
        let header = headers
            .next()
            .and_then(|h| h.to_str().ok())
            .ok_or(AppError::Unauthorized)?;
        if headers.next().is_some() {
            return Err(AppError::Unauthorized);
        }
        let (scheme, token) = header.split_once(' ').ok_or(AppError::Unauthorized)?;
        if !scheme.eq_ignore_ascii_case("Bearer")
            || token.is_empty()
            || token.len() > 4096
            || token.bytes().any(|b| b.is_ascii_whitespace())
        {
            return Err(AppError::Unauthorized);
        }
        Ok(Self(state.auth.authenticate_access(token).await?))
    }
}
