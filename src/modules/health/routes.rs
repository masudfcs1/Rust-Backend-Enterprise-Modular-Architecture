use crate::{
    bootstrap::AppState,
    database,
    shared::{ApiResponse, AppError},
};
use axum::{Router, extract::State, routing::get};
use serde::Serialize;
use std::time::Duration;

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
}

async fn live() -> ApiResponse<Health> {
    ApiResponse::new(Health { status: "ok" })
}
async fn ready(State(state): State<AppState>) -> Result<ApiResponse<Health>, AppError> {
    tokio::time::timeout(Duration::from_secs(2), database::ready(&state.pool))
        .await
        .map_err(|_| AppError::Unavailable)??;
    Ok(ApiResponse::new(Health { status: "ready" }))
}
