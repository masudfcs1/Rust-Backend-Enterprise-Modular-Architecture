use crate::{
    config::Config,
    middleware::request_id,
    modules::{auth::AuthService, users::UsersService},
    shared::AppError,
};
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderName, Method, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub users: UsersService,
    pub auth: AuthService,
    pub config: Arc<Config>,
}

impl AppState {
    pub async fn new(pool: PgPool, config: Config) -> Result<Self, AppError> {
        let users = UsersService::new(pool.clone());
        let auth = AuthService::new(pool.clone(), users.clone(), config.auth.clone()).await?;
        Ok(Self {
            pool,
            users,
            auth,
            config: Arc::new(config),
        })
    }
}

pub fn build_app(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(state.config.cors_allowed_origins.clone())
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::OPTIONS])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers([HeaderName::from_static("x-request-id")])
        .max_age(Duration::from_secs(600));
    super::routes::routes(&state)
        .layer(DefaultBodyLimit::max(state.config.request_body_limit_bytes))
        .layer(middleware::from_fn_with_state(
            Duration::from_secs(state.config.request_timeout_seconds),
            timeout,
        ))
        .layer(cors)
        .layer(middleware::from_fn(request_id::observe))
        .with_state(state)
}

async fn timeout(State(duration): State<Duration>, request: Request, next: Next) -> Response {
    match tokio::time::timeout(duration, next.run(request)).await {
        Ok(response) => response,
        Err(_) => AppError::Timeout.into_response(),
    }
}
