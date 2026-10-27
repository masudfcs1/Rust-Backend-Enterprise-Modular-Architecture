use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

use crate::{
    bootstrap::AppState,
    middleware::auth::AuthenticatedUser,
    shared::{ApiResponse, AppError, ValidatedJson},
};

use super::dto::{LoginRequest, RefreshRequest, RegisterRequest, TokenResponse};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/logout", post(logout))
}

async fn register(
    State(state): State<AppState>,
    ValidatedJson(input): ValidatedJson<RegisterRequest>,
) -> Result<(StatusCode, Json<ApiResponse<TokenResponse>>), AppError> {
    let response = state.auth.register(input).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::new(response))))
}

async fn login(
    State(state): State<AppState>,
    ValidatedJson(input): ValidatedJson<LoginRequest>,
) -> Result<Json<ApiResponse<TokenResponse>>, AppError> {
    let response = state.auth.login(input).await?;
    Ok(Json(ApiResponse::new(response)))
}

async fn refresh(
    State(state): State<AppState>,
    ValidatedJson(input): ValidatedJson<RefreshRequest>,
) -> Result<Json<ApiResponse<TokenResponse>>, AppError> {
    let response = state.auth.refresh(input.refresh_token).await?;
    Ok(Json(ApiResponse::new(response)))
}

async fn logout(
    State(state): State<AppState>,
    AuthenticatedUser(principal): AuthenticatedUser,
) -> Result<StatusCode, AppError> {
    state.auth.logout(&principal).await?;
    Ok(StatusCode::NO_CONTENT)
}
