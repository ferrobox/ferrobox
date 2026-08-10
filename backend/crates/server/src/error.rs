//! Traducción de los errores de la capa de aplicación a respuestas HTTP.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ferrobox_application::create_repository::CreateRepositoryError;
use ferrobox_application::download_artifact::DownloadArtifactError;
use ferrobox_application::get_repository::GetRepositoryError;
use ferrobox_application::packaging::PackagingError;
use ferrobox_application::publish_artifact::PublishArtifactError;
use ferrobox_ports::artifact_store::ArtifactStoreError;
use ferrobox_ports::repository_store::RepositoryStoreError;

use crate::dto::ErrorResponse;

/// Un error de la API HTTP, ya traducido a un código de estado.
pub(crate) enum ApiError {
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

impl From<RepositoryStoreError> for ApiError {
    fn from(err: RepositoryStoreError) -> Self {
        match err {
            RepositoryStoreError::DuplicateName(_) => Self::Conflict(err.to_string()),
            RepositoryStoreError::Backend(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<GetRepositoryError> for ApiError {
    fn from(err: GetRepositoryError) -> Self {
        match err {
            GetRepositoryError::NotFound(_) => Self::NotFound(err.to_string()),
            GetRepositoryError::Persistence(inner) => inner.into(),
        }
    }
}

impl From<PackagingError> for ApiError {
    fn from(err: PackagingError) -> Self {
        match err {
            PackagingError::EcosystemMismatch { .. } | PackagingError::InvalidPayload(_) => {
                Self::BadRequest(err.to_string())
            }
            PackagingError::AlreadyPublished(_) => Self::Conflict(err.to_string()),
            PackagingError::PackageNotFound(_) | PackagingError::VersionNotFound(_) => {
                Self::NotFound(err.to_string())
            }
            PackagingError::Storage(_)
            | PackagingError::ArtifactPersistence(_)
            | PackagingError::IndexPersistence(_) => Self::Internal(err.to_string()),
        }
    }
}
