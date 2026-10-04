//! HTTP routes to manage repositories: create, list, detail, and
//! delete.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::group::RepositoryAccess;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::{Repository, RepositoryKind, RepositoryName};
use ferrobox_domain::user::User;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{
    require_repo_read, require_repo_write, require_token_read, require_write_artifacts,
};
use crate::dto::{
    CreateRepositoryKindDto, CreateRepositoryRequest, CreateRepositoryResponse,
    MirrorUpstreamAuthResponse, MirrorUpstreamProbeResponse, RepositoryResponse,
    SetMirrorScheduleRequest, SetMirrorUpstreamAuthRequest, SetMirrorUpstreamRequest,
    UpdateAlloyMembersRequest,
};
use crate::error::ApiError;
use ferrobox_application::create_repository::CreateRepositoryKind;

pub(crate) async fn create_repository(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<CreateRepositoryResponse>), ApiError> {
    require_write_artifacts(&user, &token)?;

    let name =
        RepositoryName::parse(payload.name).map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let ecosystem = payload.ecosystem;

    let kind = match payload.kind.unwrap_or_default() {
        CreateRepositoryKindDto::Forge => CreateRepositoryKind::Forge,
        CreateRepositoryKindDto::Mirror { upstream } => CreateRepositoryKind::Mirror { upstream },
        CreateRepositoryKindDto::Alloy { members } => CreateRepositoryKind::Alloy {
            members: parse_alloy_member_ids(members)?,
        },
    };

    let id = state
        .create_repository
        .execute(name.clone(), ecosystem.into(), kind)
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::RepositoryCreated,
        AuditTargetKind::Repository,
        name.to_string(),
        ferrobox_domain::package_coordinate::PackageEcosystem::from(ecosystem).label(),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(CreateRepositoryResponse { id: id.to_string() }),
    ))
}

pub(crate) async fn list_repositories(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
) -> Result<Json<Vec<RepositoryResponse>>, ApiError> {
    require_token_read(&token)?;
    let visibility = state.groups.visibility(&user).await?;
    let repositories = state.list_repositories.execute().await?;
    let mut responses = Vec::new();
    for repository in repositories {
        if !visibility.contains(repository.id()) || !token.repositories().allows(repository.id()) {
            continue;
        }
        responses.push(to_repository_response(&state, &user, &repository).await?);
    }
    Ok(Json(responses))
}

pub(crate) async fn get_repository(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<RepositoryResponse>, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;
    require_repo_read(&state.groups, &user, &token, repository.id()).await?;
    Ok(Json(
        to_repository_response(&state, &user, &repository).await?,
    ))
}

pub(crate) async fn update_alloy_members(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<UpdateAlloyMembersRequest>,
) -> Result<Json<RepositoryResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, &token, repository_id).await?;

    let members = parse_alloy_member_ids(payload.members)?;
    let member_count = members.len();
    let repository = state
        .update_alloy_members
        .execute(repository_id, members)
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::RepositoryMembersChanged,
        AuditTargetKind::Repository,
        repository.name().to_string(),
        format!("{member_count} members"),
    )
    .await;

    Ok(Json(
        to_repository_response(&state, &user, &repository).await?,
    ))
}

pub(crate) async fn set_mirror_schedule(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<SetMirrorScheduleRequest>,
) -> Result<Json<RepositoryResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, &token, repository_id).await?;

    let repository = ferrobox_application::mirror_schedule::SetMirrorScheduleUseCase::execute(
        &state.get_repository,
        repository_id,
        payload.prefetch_interval_hours,
    )
    .await?;

    let detail = match repository.prefetch_interval_hours() {
        Some(hours) => format!("every {hours}h"),
        None => "disabled".to_string(),
    };
    crate::audit::record(
        &state,
        &user,
        AuditAction::MirrorScheduleChanged,
        AuditTargetKind::Repository,
        repository.name().to_string(),
        detail,
    )
    .await;

    Ok(Json(
        to_repository_response(&state, &user, &repository).await?,
    ))
}

pub(crate) async fn set_mirror_upstream(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<SetMirrorUpstreamRequest>,
) -> Result<Json<RepositoryResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, &token, repository_id).await?;

    let repository = ferrobox_application::mirror_upstream::SetMirrorUpstreamUseCase::execute(
        &state.get_repository,
        repository_id,
        &payload.upstream,
    )
    .await?;

    let detail = match repository.kind() {
        RepositoryKind::Mirror { upstream } => upstream.to_string(),
        RepositoryKind::Forge | RepositoryKind::Alloy { .. } => payload.upstream,
    };
    crate::audit::record(
        &state,
        &user,
        AuditAction::MirrorUpstreamUrlChanged,
        AuditTargetKind::Repository,
        repository.name().to_string(),
        detail,
    )
    .await;

    Ok(Json(
        to_repository_response(&state, &user, &repository).await?,
    ))
}

pub(crate) async fn get_upstream_auth(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<MirrorUpstreamAuthResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_read(&state.groups, &user, &token, repository_id).await?;
    let status = state.mirror_credentials.status(repository_id).await?;
    Ok(Json(MirrorUpstreamAuthResponse {
        configured: status.configured,
        username: status.username,
    }))
}

pub(crate) async fn set_upstream_auth(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<SetMirrorUpstreamAuthRequest>,
) -> Result<Json<MirrorUpstreamAuthResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, &token, repository_id).await?;
    let status = state
        .mirror_credentials
        .save(repository_id, payload.username, payload.secret)
        .await?;
    let repository = state.get_repository.execute(repository_id).await?;
    let detail = if status.username.is_empty() {
        "bearer".to_string()
    } else {
        status.username.clone()
    };
    crate::audit::record(
        &state,
        &user,
        AuditAction::MirrorUpstreamChanged,
        AuditTargetKind::Repository,
        repository.name().to_string(),
        detail,
    )
    .await;
    Ok(Json(MirrorUpstreamAuthResponse {
        configured: status.configured,
        username: status.username,
    }))
}

pub(crate) async fn test_upstream_auth(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<MirrorUpstreamProbeResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, &token, repository_id).await?;
    let probe = state.mirror_credentials.probe(repository_id).await?;
    Ok(Json(MirrorUpstreamProbeResponse {
        status: probe.status,
        ok: probe.ok,
        authenticated: probe.authenticated,
    }))
}

pub(crate) async fn clear_upstream_auth(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, &token, repository_id).await?;
    let repository = state.get_repository.execute(repository_id).await?;
    state.mirror_credentials.clear(repository_id).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::MirrorUpstreamChanged,
        AuditTargetKind::Repository,
        repository.name().to_string(),
        "cleared".to_string(),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_repository(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, token }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, &token, repository_id).await?;
    state.worm.ensure_mutable(repository_id).await?;
    let repository = state.get_repository.execute(repository_id).await?;

    state.delete_repository.execute(repository_id).await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::RepositoryDeleted,
        AuditTargetKind::Repository,
        repository.name().to_string(),
        "",
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

async fn to_repository_response(
    state: &AppState,
    user: &User,
    repository: &Repository,
) -> Result<RepositoryResponse, ApiError> {
    let access = state
        .groups
        .access_on(user, repository.id())
        .await?
        .unwrap_or(RepositoryAccess::Read);
    let restricted = state.groups.is_restricted(repository.id()).await?;
    Ok(RepositoryResponse::from_repository(
        repository, access, restricted,
    ))
}

fn parse_alloy_member_ids(members: Vec<String>) -> Result<Vec<RepositoryId>, ApiError> {
    let mut parsed = Vec::with_capacity(members.len());
    for member in members {
        let uuid = Uuid::parse_str(&member)
            .map_err(|_| ApiError::BadRequest("invalid alloy member repository id".to_string()))?;
        parsed.push(RepositoryId::from(uuid));
    }
    Ok(parsed)
}
