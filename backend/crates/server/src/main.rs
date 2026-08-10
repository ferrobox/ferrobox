//! Punto de composición de `FerroBox`: el único lugar del proyecto que
//! conoce todas las implementaciones concretas de cada puerto.

mod config;

use std::net::SocketAddr;
use std::sync::Arc;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use config::Config;
use ferrobox_adapter_postgres::artifact_store::PostgresArtifactStore;
use ferrobox_adapter_postgres::repository_store::PostgresRepositoryStore;
use ferrobox_adapter_s3_storage::S3StorageAdapter;
use ferrobox_application::create_repository::{CreateRepositoryError, CreateRepositoryUseCase};
use ferrobox_application::download_artifact::{DownloadArtifactError, DownloadArtifactUseCase};
use ferrobox_application::list_repository_artifacts::ListRepositoryArtifactsUseCase;
use ferrobox_application::publish_artifact::{PublishArtifactError, PublishArtifactUseCase};
use ferrobox_domain::artifact::Artifact;
use ferrobox_domain::ids::{ArtifactId, RepositoryId};
use ferrobox_domain::repository::RepositoryName;
use ferrobox_ports::artifact_store::ArtifactStoreError;
use ferrobox_ports::repository_store::RepositoryStoreError;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

/// Estado compartido por todos los manejadores de rutas.
struct AppState {
    create_repository: CreateRepositoryUseCase,
    publish_artifact: PublishArtifactUseCase,
    download_artifact: DownloadArtifactUseCase,
    list_repository_artifacts: ListRepositoryArtifactsUseCase,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let config = Config::from_env().expect("invalid configuration");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("failed to connect to PostgreSQL");

    let s3_client = build_s3_client(&config);

    let repository_store = Arc::new(PostgresRepositoryStore::new(pool.clone()));
    let artifact_store = Arc::new(PostgresArtifactStore::new(pool));
    let storage = Arc::new(S3StorageAdapter::new(s3_client, config.s3_bucket.clone()));

    let state = Arc::new(AppState {
        create_repository: CreateRepositoryUseCase::new(repository_store.clone()),
        publish_artifact: PublishArtifactUseCase::new(
            repository_store,
            artifact_store.clone(),
            storage.clone(),
        ),
        download_artifact: DownloadArtifactUseCase::new(artifact_store.clone(), storage),
        list_repository_artifacts: ListRepositoryArtifactsUseCase::new(artifact_store),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/repositories", post(create_repository))
        .route(
            "/repositories/{repository_id}/artifacts",
            post(publish_artifact).get(list_repository_artifacts),
        )
        .route("/artifacts/{artifact_id}", get(download_artifact))
        .with_state(state);

    let addr: SocketAddr = config.bind_address.parse().expect("invalid bind address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to address");

    println!("FerroBox escuchando en http://{addr}");

    axum::serve(listener, app).await.expect("server error");
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize)]
struct CreateRepositoryRequest {
    name: String,
}

#[derive(Serialize)]
struct CreateRepositoryResponse {
    id: String,
}

async fn create_repository(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<CreateRepositoryResponse>), ApiError> {
    let name =
        RepositoryName::parse(payload.name).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let id = state.create_repository.execute(name).await?;

    Ok((
        StatusCode::CREATED,
        Json(CreateRepositoryResponse { id: id.to_string() }),
    ))
}

#[derive(Serialize)]
struct PublishResponse {
    id: String,
}

async fn publish_artifact(
    State(state): State<Arc<AppState>>,
    Path(repository_id): Path<Uuid>,
    body: Bytes,
) -> Result<(StatusCode, Json<PublishResponse>), ApiError> {
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

async fn download_artifact(
    State(state): State<Arc<AppState>>,
    Path(artifact_id): Path<Uuid>,
) -> Result<Bytes, ApiError> {
    let (_artifact, content) = state
        .download_artifact
        .execute(ArtifactId::from(artifact_id))
        .await?;

    Ok(content)
}

#[derive(Serialize)]
struct ArtifactResponse {
    id: String,
    checksum: String,
    size_bytes: u64,
}

impl From<Artifact> for ArtifactResponse {
    fn from(artifact: Artifact) -> Self {
        Self {
            id: artifact.id().to_string(),
            checksum: artifact.checksum().to_string(),
            size_bytes: artifact.size_bytes(),
        }
    }
}

async fn list_repository_artifacts(
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

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

enum ApiError {
    BadRequest(String),
    Conflict(String),
    NotFound(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Conflict(message) => (StatusCode::CONFLICT, message),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, message),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

impl From<CreateRepositoryError> for ApiError {
    fn from(err: CreateRepositoryError) -> Self {
        match &err {
            CreateRepositoryError::Persistence(RepositoryStoreError::DuplicateName(_)) => {
                Self::Conflict(err.to_string())
            }
            CreateRepositoryError::Persistence(RepositoryStoreError::Backend(_)) => {
                Self::Internal(err.to_string())
            }
        }
    }
}

impl From<PublishArtifactError> for ApiError {
    fn from(err: PublishArtifactError) -> Self {
        match err {
            PublishArtifactError::RepositoryNotFound(_) => Self::NotFound(err.to_string()),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<DownloadArtifactError> for ApiError {
    fn from(err: DownloadArtifactError) -> Self {
        match err {
            DownloadArtifactError::NotFound(_) => Self::NotFound(err.to_string()),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<ArtifactStoreError> for ApiError {
    fn from(err: ArtifactStoreError) -> Self {
        Self::Internal(err.to_string())
    }
}

fn build_s3_client(config: &Config) -> S3Client {
    let credentials = Credentials::new(
        &config.s3_access_key_id,
        &config.s3_secret_access_key,
        None,
        None,
        "ferrobox-server",
    );

    let s3_config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(config.s3_region.clone()))
        .endpoint_url(&config.s3_endpoint_url)
        .credentials_provider(credentials)
        .force_path_style(true)
        .build();

    S3Client::from_conf(s3_config)
}
