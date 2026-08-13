//! Rutas HTTP para gestionar repositorios: creación, listado, detalle y
//! eliminación.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::RepositoryName;
use uuid::Uuid;

use crate::AppState;
use crate::auth_extract::AuthenticatedUser;
use crate::authz::require_write_artifacts;
use crate::dto::{
    CreateRepositoryKindDto, CreateRepositoryRequest, CreateRepositoryResponse, RepositoryResponse,
};
use crate::error::ApiError;
use ferrobox_application::create_repository::CreateRepositoryKind;

pub(crate) async fn create_repository(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<CreateRepositoryResponse>), ApiError> {
    require_write_artifacts(&user)?;

    let name =
        RepositoryName::parse(payload.name).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let kind = match payload.kind.unwrap_or_default() {
        CreateRepositoryKindDto::Forge => CreateRepositoryKind::Forge,
        CreateRepositoryKindDto::Mirror { upstream } => CreateRepositoryKind::Mirror { upstream },
    };

    let id = state
        .create_repository
        .execute(name, payload.ecosystem.into(), kind)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(CreateRepositoryResponse { id: id.to_string() }),
    ))
}

pub(crate) async fn list_repositories(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RepositoryResponse>>, ApiError> {
    let repositories = state.list_repositories.execute().await?;

    Ok(Json(
        repositories.iter().map(RepositoryResponse::from).collect(),
    ))
}

pub(crate) async fn get_repository(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
) -> Result<Json<RepositoryResponse>, ApiError> {
    let repository = state
        .get_repository
        .execute(RepositoryId::from(repository_id))
        .await?;

    Ok(Json(RepositoryResponse::from(&repository)))
}

pub(crate) async fn delete_repository(
    State(state): State<Arc<AppState>>,
    AuthenticatedUser { user, .. }: AuthenticatedUser,
    Path(repository_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_write_artifacts(&user)?;

    state
        .delete_repository
        .execute(RepositoryId::from(repository_id))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
