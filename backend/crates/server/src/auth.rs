//! Rutas HTTP de autenticación: login, perfil y gestión de tokens.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::api_token::ApiTokenName;
use ferrobox_domain::ids::ApiTokenId;
use ferrobox_domain::user::Username;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::dto::{
    ApiTokenCreatedResponse, ApiTokenResponse, CreateApiTokenRequest, LoginRequest, LoginResponse,
    UserResponse,
};
use crate::error::ApiError;

pub(crate) async fn login(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let username =
        Username::parse(payload.username).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let result = state.login.execute(username, &payload.password).await?;

    Ok(Json(LoginResponse {
        token: result.plaintext_secret,
        user: UserResponse {
            id: result.user.id().to_string(),
            username: result.user.username().to_string(),
        },
    }))
}

pub(crate) async fn me(
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Json<UserResponse> {
    Json(UserResponse {
        id: user.id().to_string(),
        username: user.username().to_string(),
    })
}

pub(crate) async fn list_tokens(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
) -> Result<Json<Vec<ApiTokenResponse>>, ApiError> {
    let tokens = state.list_api_tokens.execute(user.id()).await?;

    Ok(Json(
        tokens
            .into_iter()
            .map(|record| ApiTokenResponse {
                id: record.token.id().to_string(),
                name: record.token.name().to_string(),
                prefix: record.token.prefix().to_string(),
                created_at: record.created_at_rfc3339,
            })
            .collect(),
    ))
}

pub(crate) async fn create_token(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Json(payload): Json<CreateApiTokenRequest>,
) -> Result<(StatusCode, Json<ApiTokenCreatedResponse>), ApiError> {
    let name =
        ApiTokenName::parse(payload.name).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let result = state.create_api_token.execute(user.id(), name).await?;

    Ok((
        StatusCode::CREATED,
        Json(ApiTokenCreatedResponse {
            id: result.token.id().to_string(),
            name: result.token.name().to_string(),
            prefix: result.token.prefix().to_string(),
            token: result.plaintext_secret,
        }),
    ))
}

pub(crate) async fn revoke_token(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(token_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state
        .revoke_api_token
        .execute(user.id(), ApiTokenId::from(token_id))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
