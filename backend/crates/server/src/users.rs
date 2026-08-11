//! Rutas HTTP de administración de usuarios (solo rol `Admin`).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::ids::UserId;
use ferrobox_domain::user::Username;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_manage_users;
use crate::dto::{CreateUserRequest, UserResponse};
use crate::error::ApiError;

pub(crate) async fn list_users(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    require_manage_users(&user)?;
    let users = state.list_users.execute().await?;
    Ok(Json(users.iter().map(UserResponse::from).collect()))
}

pub(crate) async fn create_user(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Json(payload): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    require_manage_users(&user)?;

    let username =
        Username::parse(payload.username).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    if payload.password.is_empty() {
        return Err(ApiError::BadRequest(
            "password cannot be empty".to_string(),
        ));
    }

    let created = state
        .create_user
        .execute(username, &payload.password, payload.role.into())
        .await?;

    Ok((StatusCode::CREATED, Json(UserResponse::from(&created))))
}

pub(crate) async fn delete_user(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_manage_users(&user)?;

    state
        .delete_user
        .execute(user.id(), UserId::from(user_id))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
