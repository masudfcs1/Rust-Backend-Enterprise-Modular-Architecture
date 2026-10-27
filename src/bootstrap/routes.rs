use super::AppState;
use crate::{
    middleware::rate_limit::{self, AuthRateLimiter},
    modules::{auth, health, users},
    shared::AppError,
};
use axum::{Router, middleware};

pub fn routes(state: &AppState) -> Router<AppState> {
    let auth_routes = auth::routes().layer(middleware::from_fn_with_state(
        AuthRateLimiter::new(state.config.auth_requests_per_minute),
        rate_limit::enforce,
    ));
    Router::new()
        .merge(health::routes())
        .nest("/api/v1/auth", auth_routes)
        .nest("/api/v1/users", users::routes())
        .fallback(|| async { AppError::NotFound("Route not found") })
        .method_not_allowed_fallback(|| async { AppError::MethodNotAllowed })
}
