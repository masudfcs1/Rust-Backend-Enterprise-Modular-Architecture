use axum::{Router, routing::get};

use crate::bootstrap::AppState;

use super::handler;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(handler::list))
        .route("/me", get(handler::me).patch(handler::update_me))
        .route("/{id}", get(handler::get_by_id))
}
