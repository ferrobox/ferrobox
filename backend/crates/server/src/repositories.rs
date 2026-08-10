//! Rutas HTTP para gestionar repositorios: creación, listado y detalle.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::repository::RepositoryName;
use uuid::Uuid;

use crate::AppState;
use crate::dto::{CreateRepositoryRequest, CreateRepositoryResponse, RepositoryResponse};
use crate::error::ApiError;

pub(crate) async fn create_repository(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<CreateRepositoryResponse>), ApiError> {
    let name =
        RepositoryName::parse(payload.name).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let id = state
        .create_repository
        .execute(name, payload.ecosystem.into())
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
