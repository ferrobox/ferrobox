//! Rutas HTTP de autenticación: login, perfil y gestión de tokens.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::api_token::ApiTokenName;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::ApiTokenId;
use ferrobox_domain::user::Username;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::dto::{
    ApiTokenCreatedResponse, ApiTokenResponse, ChangePasswordRequest, CreateApiTokenRequest,
    LoginRequest, LoginResponse, UserResponse,
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
        user: UserResponse::from(&result.user),
    }))
}

pub(crate) async fn me(AuthenticatedUser { user, .. }: AuthenticatedUser) -> Json<UserResponse> {
    Json(UserResponse::from(&user))
}

pub(crate) async fn change_password(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    if payload.new_password.is_empty() {
        return Err(ApiError::BadRequest("password cannot be empty".to_string()));
    }

    state
        .change_password
        .execute(
            user.username(),
            &payload.current_password,
            &payload.new_password,
        )
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::UserPasswordChanged,
        AuditTargetKind::User,
        user.username().to_string(),
        "",
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
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

    crate::audit::record(
        &state,
        &user,
        AuditAction::TokenCreated,
        AuditTargetKind::Token,
        result.token.name().to_string(),
        result.token.prefix().to_string(),
    )
    .await;

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

    crate::audit::record(
        &state,
        &user,
        AuditAction::TokenRevoked,
        AuditTargetKind::Token,
        token_id.to_string(),
        "",
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
