//! Rutas HTTP para publicar, listar y descargar artefactos genéricos
//! (sin ningún protocolo de ecosistema de por medio).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use bytes::Bytes;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_write_artifacts;
use crate::dto::{ArtifactResponse, PublishResponse};
use crate::error::ApiError;

pub(crate) async fn publish_artifact(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
    body: Bytes,
) -> Result<(StatusCode, Json<PublishResponse>), ApiError> {
    require_write_artifacts(&user)?;

    let artifact_id = state
        .publish_artifact
        .execute(RepositoryId::from(repository_id), body)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(PublishResponse {
            id: artifact_id.to_string(),
        }),
    ))
}

pub(crate) async fn download_artifact(
    State(state): State<Arc<AppState>>,
    Path(artifact_id): Path<Uuid>,
) -> Result<Bytes, ApiError> {
    let (_artifact, content) = state
        .download_artifact
        .execute(ArtifactId::from(artifact_id))
        .await?;

    Ok(content)
}

pub(crate) async fn list_repository_artifacts(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<Vec<ArtifactResponse>>, ApiError> {
    let artifacts = state
        .list_repository_artifacts
        .execute(RepositoryId::from(repository_id))
        .await?;

    Ok(Json(
        artifacts.into_iter().map(ArtifactResponse::from).collect(),
    ))
}
