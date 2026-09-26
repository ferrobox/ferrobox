//! Rutas HTTP para publicar, listar y descargar artefactos genéricos
//! (sin ningún protocolo de ecosistema de por medio).

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, HeaderValue};
use axum::http::StatusCode;
use axum::response::Response;
use bytes::Bytes;
use ferrobox_domain::audit::{AuditAction, AuditTargetKind};
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::{require_repo_read, require_repo_write};
use crate::dto::{
    ArtifactResponse, ImportRepositoryResponse, PrefetchPackageRequest, PrefetchPackageResponse,
    PromotePackageRequest, PromotePackageResponse, PublishResponse,
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

pub(crate) async fn prefetch_package(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    Json(payload): Json<PrefetchPackageRequest>,
) -> Result<Json<PrefetchPackageResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;

    let name = ferrobox_domain::package_coordinate::PackageName::parse(payload.name.trim())
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let version = match payload.version.as_deref().map(str::trim).filter(|item| !item.is_empty())
    {
        Some(value) => Some(
            ferrobox_domain::package_coordinate::PackageVersion::parse(value)
                .map_err(|err| ApiError::BadRequest(err.to_string()))?,
        ),
        None => None,
    };

    let repository = state.get_repository.execute(repository_id).await?;
    let outcome = ferrobox_application::prefetch_package::PrefetchPackageUseCase::execute(
        &state.packaging,
        &repository,
        name,
        version,
    )
    .await?;

    let audit_target = match &outcome.version {
        Some(version) => format!("{}@{version}", outcome.name),
        None => outcome.name.clone(),
    };
    crate::audit::record(
        &state,
        &user,
        AuditAction::PackagePrefetched,
        AuditTargetKind::Package,
        audit_target,
        repository_id.to_string(),
    )
    .await;

    Ok(Json(PrefetchPackageResponse {
        name: outcome.name,
        version: outcome.version,
        indexed: outcome.indexed,
        downloaded: outcome.downloaded,
    }))
}

pub(crate) async fn export_repository(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_read(&state.groups, &user, repository_id).await?;

    let bundle = state.repository_bundle.export(repository_id).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::RepositoryExported,
        AuditTargetKind::Repository,
        bundle.filename.clone(),
        format!(
            "{} packages, {} artifacts",
            bundle.packages, bundle.artifacts
        ),
    )
    .await;

    let disposition = format!("attachment; filename=\"{}\"", bundle.filename);
    let mut response = Response::new(Body::from(bundle.bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/gzip"),
    );
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition)
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok(response)
}

pub(crate) async fn import_repository(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<ImportRepositoryResponse>, ApiError> {
    let repository_id = RepositoryId::from(repository_id);
    require_repo_write(&state.groups, &user, repository_id).await?;

    let mut bundle = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?
    {
        if field.name() == Some("bundle") {
            let bytes = field
                .bytes()
                .await
                .map_err(|err| ApiError::BadRequest(err.to_string()))?;
            bundle = Some(bytes);
        }
    }
    let bundle = bundle.ok_or_else(|| {
        ApiError::BadRequest("multipart field 'bundle' is required".to_string())
    })?;

    let outcome = state.repository_bundle.import(repository_id, bundle).await?;
    crate::audit::record(
        &state,
        &user,
        AuditAction::RepositoryImported,
        AuditTargetKind::Repository,
        repository_id.to_string(),
        format!(
            "{} packages, {} artifacts, {} skipped",
            outcome.packages_imported, outcome.artifacts_imported, outcome.skipped
        ),
    )
    .await;

    Ok(Json(ImportRepositoryResponse {
        packages_imported: outcome.packages_imported,
        artifacts_imported: outcome.artifacts_imported,
        skipped: outcome.skipped,
        bytes_copied: outcome.bytes_copied,
    }))
}
