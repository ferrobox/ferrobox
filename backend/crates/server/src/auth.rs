//! HTTP routes for authentication: login, profile, and token management.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::api_token::{ApiTokenName, TokenRepositories, TokenScopes};
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::{ApiTokenId, RepositoryId};
use ferrobox_domain::user::Username;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_token_write, require_unrestricted_repositories};
use crate::dto::{
    ApiTokenCreatedResponse, ApiTokenResponse, ChangePasswordRequest, CreateApiTokenRequest,
    LoginRequest, LoginResponse, UserResponse, token_repository_ids, token_scope_labels,
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
    require_unrestricted_repositories(&token)?;
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
    AuthenticatedUser { user, token }: AuthenticatedUser,
) -> Result<Json<Vec<ApiTokenResponse>>, ApiError> {
    require_unrestricted_repositories(&token)?;
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
                repository_ids: token_repository_ids(&record.token),
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
    require_unrestricted_repositories(&token)?;
    let name =
        ApiTokenName::parse(payload.name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let scopes = TokenScopes::parse(payload.scopes.unwrap_or_default())
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let repositories = parse_repository_scope(&state, &user, payload.repository_ids).await?;

    let expires_at = parse_optional_expiry(payload.expires_at.as_deref())?;
    let result = state
        .create_api_token
        .execute_limited(user.id(), name, expires_at, scopes, repositories)
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
            repository_ids: token_repository_ids(&result.token),
        }),
    ))
}

const MAX_TOKEN_REPOSITORIES: usize = 100;

async fn parse_repository_scope(
    state: &AppState,
    user: &ferrobox_domain::user::User,
    raw: Option<Vec<String>>,
) -> Result<TokenRepositories, ApiError> {
    let Some(raw) = raw.filter(|ids| ids.iter().any(|id| !id.trim().is_empty())) else {
        return Ok(TokenRepositories::unrestricted());
    };
    if raw.len() > MAX_TOKEN_REPOSITORIES {
        return Err(ApiError::BadRequest(format!(
            "a token can list at most {MAX_TOKEN_REPOSITORIES} repositories"
        )));
    }
    let visibility = state.groups.visibility(user).await?;
    let mut ids = Vec::new();
    for raw_id in raw {
        let raw_id = raw_id.trim();
        if raw_id.is_empty() {
            continue;
        }
        let id = Uuid::parse_str(raw_id)
            .map(RepositoryId::from)
            .map_err(|err| ApiError::BadRequest(format!("invalid repository id: {err}")))?;
        state.get_repository.execute(id).await?;
        if !visibility.contains(id) {
            return Err(ApiError::Forbidden(
                "you do not have access to this repository".to_string(),
            ));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(TokenRepositories::from_ids(ids))
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
    require_unrestricted_repositories(&token)?;
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
