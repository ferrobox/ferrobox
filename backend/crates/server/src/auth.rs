//! Rutas HTTP de autenticación: login, perfil y gestión de tokens.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::api_token::{ApiTokenName, TokenScopes};
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::ApiTokenId;
use ferrobox_domain::user::Username;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_token_write;
use crate::dto::{
    ApiTokenCreatedResponse, ApiTokenResponse, ChangePasswordRequest, CreateApiTokenRequest,
    LoginRequest, LoginResponse, UserResponse, token_scope_labels,
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
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    require_token_write(&token)?;
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
                expires_at: record.expires_at_rfc3339,
                scopes: token_scope_labels(&record.token),
            })
            .collect(),
    ))
}

pub(crate) async fn create_token(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Json(payload): Json<CreateApiTokenRequest>,
) -> Result<(StatusCode, Json<ApiTokenCreatedResponse>), ApiError> {
    require_token_write(&token)?;
    let name =
        ApiTokenName::parse(payload.name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let scopes = TokenScopes::parse(payload.scopes.unwrap_or_default())
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let expires_at = parse_optional_expiry(payload.expires_at.as_deref())?;
    let result = state
        .create_api_token
        .execute_with_scopes(user.id(), name, expires_at, scopes)
        .await?;

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
            expires_at: result.token.expires_at().map(|at| at.to_rfc3339()),
            scopes: token_scope_labels(&result.token),
        }),
    ))
}

pub(crate) fn parse_optional_expiry(
    value: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|item| !item.is_empty()) else {
        return Ok(None);
    };
    let parsed = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|err| ApiError::BadRequest(format!("invalid expires_at: {err}")))?;
    Ok(Some(parsed.with_timezone(&chrono::Utc)))
}

pub(crate) async fn revoke_token(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(token_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_token_write(&token)?;
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
