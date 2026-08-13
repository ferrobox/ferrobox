//! Traducción de los errores de la capa de aplicación a respuestas HTTP.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ferrobox_application::authenticate_token::AuthenticateTokenError;
use ferrobox_application::create_repository::CreateRepositoryError;
use ferrobox_application::download_artifact::DownloadArtifactError;
use ferrobox_application::get_repository::GetRepositoryError;
use ferrobox_application::login::LoginError;
use ferrobox_application::manage_api_tokens::{
    CreateApiTokenError, ListApiTokensError, RevokeApiTokenError,
};
use ferrobox_application::manage_users::{
    ChangeUserRoleError, CreateUserError, DeleteUserError, ListUsersError,
};
use ferrobox_application::packaging::PackagingError;
use ferrobox_application::publish_artifact::PublishArtifactError;
use ferrobox_ports::artifact_store::ArtifactStoreError;
use ferrobox_ports::repository_store::RepositoryStoreError;

use crate::dto::ErrorResponse;

/// Un error de la API HTTP, ya traducido a un código de estado.
pub(crate) enum ApiError {
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    Conflict(String),
    NotFound(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message),
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
            CreateRepositoryError::UnsupportedMirrorEcosystem
            | CreateRepositoryError::InvalidUpstream(_) => Self::BadRequest(err.to_string()),
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
            PackagingError::EcosystemMismatch { .. }
            | PackagingError::InvalidPayload(_)
            | PackagingError::ReadOnlyRepository
            | PackagingError::InvalidUpstream(_) => Self::BadRequest(err.to_string()),
            PackagingError::AlreadyPublished(_) => Self::Conflict(err.to_string()),
            PackagingError::PackageNotFound(_) | PackagingError::VersionNotFound(_) => {
                Self::NotFound(err.to_string())
            }
            PackagingError::Upstream(ferrobox_ports::http_client::HttpClientError::Status {
                status: 404,
                ..
            }) => Self::NotFound(err.to_string()),
            PackagingError::Storage(_)
            | PackagingError::ArtifactPersistence(_)
            | PackagingError::IndexPersistence(_)
            | PackagingError::Upstream(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<LoginError> for ApiError {
    fn from(err: LoginError) -> Self {
        match err {
            LoginError::InvalidCredentials => Self::Unauthorized(err.to_string()),
            LoginError::TokenPersistence(_) | LoginError::UserPersistence(_) => {
                Self::Internal(err.to_string())
            }
        }
    }
}

impl From<CreateApiTokenError> for ApiError {
    fn from(err: CreateApiTokenError) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<ListApiTokensError> for ApiError {
    fn from(err: ListApiTokensError) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<RevokeApiTokenError> for ApiError {
    fn from(err: RevokeApiTokenError) -> Self {
        match err {
            RevokeApiTokenError::NotFound => Self::NotFound(err.to_string()),
            RevokeApiTokenError::Persistence(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<AuthenticateTokenError> for ApiError {
    fn from(err: AuthenticateTokenError) -> Self {
        match err {
            AuthenticateTokenError::InvalidToken => Self::Unauthorized(err.to_string()),
            AuthenticateTokenError::TokenPersistence(_)
            | AuthenticateTokenError::UserPersistence(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<CreateUserError> for ApiError {
    fn from(err: CreateUserError) -> Self {
        match &err {
            CreateUserError::Persistence(
                ferrobox_ports::user_store::UserStoreError::DuplicateUsername(_),
            ) => Self::Conflict(err.to_string()),
            CreateUserError::PasswordHashing(_)
            | CreateUserError::Persistence(ferrobox_ports::user_store::UserStoreError::Backend(
                _,
            )) => Self::Internal(err.to_string()),
        }
    }
}

impl From<ListUsersError> for ApiError {
    fn from(err: ListUsersError) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<DeleteUserError> for ApiError {
    fn from(err: DeleteUserError) -> Self {
        match err {
            DeleteUserError::NotFound => Self::NotFound(err.to_string()),
            DeleteUserError::CannotDeleteSelf | DeleteUserError::CannotDeleteLastAdmin => {
                Self::Conflict(err.to_string())
            }
            DeleteUserError::Persistence(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<ChangeUserRoleError> for ApiError {
    fn from(err: ChangeUserRoleError) -> Self {
        match err {
            ChangeUserRoleError::NotFound => Self::NotFound(err.to_string()),
            ChangeUserRoleError::CannotDemoteLastAdmin => Self::Conflict(err.to_string()),
            ChangeUserRoleError::Persistence(_) => Self::Internal(err.to_string()),
        }
    }
}
