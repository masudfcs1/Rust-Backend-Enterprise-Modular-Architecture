use axum::{Json, extract::State};
use uuid::Uuid;

use crate::{
    bootstrap::AppState,
    middleware::auth::AuthenticatedUser,
    shared::{
        error::AppError,
        extract::{ApiPath, ApiQuery, ValidatedJson},
        pagination::{Paginated, Pagination},
        response::ApiResponse,
    },
};

use super::{
    dto::UpdateProfileRequest,
    model::{Role, User},
};

pub(super) async fn me(
    State(state): State<AppState>,
    AuthenticatedUser(principal): AuthenticatedUser,
) -> Result<Json<ApiResponse<User>>, AppError> {
    let user = state.users.get_by_id(principal.user_id).await?;
    Ok(Json(ApiResponse::new(user)))
}

pub(super) async fn update_me(
    State(state): State<AppState>,
    AuthenticatedUser(principal): AuthenticatedUser,
    ValidatedJson(input): ValidatedJson<UpdateProfileRequest>,
) -> Result<Json<ApiResponse<User>>, AppError> {
    let user = state.users.update_profile(principal.user_id, input).await?;
    Ok(Json(ApiResponse::new(user)))
}

pub(super) async fn list(
    State(state): State<AppState>,
    AuthenticatedUser(principal): AuthenticatedUser,
    ApiQuery(pagination): ApiQuery<Pagination>,
) -> Result<Json<ApiResponse<Paginated<User>>>, AppError> {
    require_admin(principal.role)?;
    let users = state.users.list(pagination).await?;
    Ok(Json(ApiResponse::new(users)))
}

pub(super) async fn get_by_id(
    State(state): State<AppState>,
    AuthenticatedUser(principal): AuthenticatedUser,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ApiResponse<User>>, AppError> {
    require_admin(principal.role)?;
    let user = state.users.get_by_id(id).await?;
    Ok(Json(ApiResponse::new(user)))
}

pub(super) fn require_admin(role: Role) -> Result<(), AppError> {
    if role != Role::Admin {
        return Err(AppError::Forbidden);
    }
    Ok(())
}
