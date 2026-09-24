//! Rutas HTTP para publicar, listar y descargar artefactos genéricos
//! (sin ningún protocolo de ecosistema de por medio).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use bytes::Bytes;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_repo_read, require_repo_write};
use crate::dto::{
    ArtifactResponse, PromotePackageRequest, PromotePackageResponse, PublishResponse,
};
use crate::error::ApiError;

pub(crate) async fn publish_artifact(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    body: Bytes,
) -> Result<(StatusCode, Json<PublishResponse>), ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;

    let artifact_id = state.publish_artifact.execute(repository_id, body).await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::ArtifactPublished,
        AuditTargetKind::Artifact,
        artifact_id.to_string(),
        repository_id.to_string(),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(PublishResponse {
            id: artifact_id.to_string(),
        }),
    ))
}

pub(crate) async fn download_artifact(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(artifact_id): Path<Uuid>,
) -> Result<Bytes, ApiError> {
    let (artifact, content) = state
        .download_artifact
        .execute(ArtifactId::from(artifact_id))
        .await?;
    require_repo_read(&state.groups, &user, artifact.repository_id()).await?;

    Ok(content)
}

pub(crate) async fn list_repository_artifacts(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<Vec<ArtifactResponse>>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_read(&state.groups, &user, repository_id).await?;

    let artifacts = state
        .list_repository_artifacts
        .execute(repository_id)
        .await?;

    Ok(Json(
        artifacts.into_iter().map(ArtifactResponse::from).collect(),
    ))
}

pub(crate) async fn delete_artifact(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path((repository_id, artifact_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;

    state
        .delete_artifact
        .execute(repository_id, ArtifactId::from(artifact_id))
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::ArtifactDeleted,
        AuditTargetKind::Artifact,
        artifact_id.to_string(),
        repository_id.to_string(),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn promote_package(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<PromotePackageRequest>,
) -> Result<(StatusCode, Json<PromotePackageResponse>), ApiError> {
    let source_id = RepositoryId::from(repository_id);
    let target_uuid = Uuid::parse_str(&payload.target_repository_id)
        .map_err(|_| ApiError::BadRequest("invalid target repository id".to_string()))?;
    let target_id = RepositoryId::from(target_uuid);
    require_repo_read(&state.groups, &user, source_id).await?;
    require_repo_write(&state.groups, &user, target_id).await?;

    let artifact_id = match payload.artifact_id.as_deref() {
        Some(value) if !value.is_empty() => {
            let uuid = Uuid::parse_str(value)
                .map_err(|_| ApiError::BadRequest("invalid artifact id".to_string()))?;
            Some(ArtifactId::from(uuid))
        }
        _ => None,
    };

    let outcome = state
        .promote_package
        .execute(
            &state.packaging,
            source_id,
            target_id,
            payload.name.as_deref(),
            payload.version.as_deref(),
            artifact_id,
            payload.preserve_yanked,
        )
        .await?;

    crate::audit::record(
        &state,
        &user,
        AuditAction::PackagePromoted,
        AuditTargetKind::Package,
        format!(
            "{}@{}",
            outcome.name.as_deref().unwrap_or("?"),
            outcome.version.as_deref().unwrap_or("?")
        ),
        target_id.to_string(),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(PromotePackageResponse {
            name: outcome.name,
            version: outcome.version,
            artifacts_copied: outcome.artifacts_copied,
            bytes_copied: outcome.bytes_copied,
        }),
    ))
}
