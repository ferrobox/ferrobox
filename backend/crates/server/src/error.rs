//! Traducción de los errores de la capa de aplicación a respuestas HTTP.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ferrobox_application::admission::AdmissionError;
use ferrobox_application::audit::AuditError;
use ferrobox_application::authenticate_token::AuthenticateTokenError;
use ferrobox_application::change_password::ChangePasswordError;
use ferrobox_application::create_repository::CreateRepositoryError;
use ferrobox_application::delete_artifact::DeleteArtifactError;
use ferrobox_application::delete_repository::DeleteRepositoryError;
use ferrobox_application::download_artifact::DownloadArtifactError;
use ferrobox_application::get_repository::GetRepositoryError;
use ferrobox_application::list_repository_artifacts::ListRepositoryArtifactsError;
use ferrobox_application::login::LoginError;
use ferrobox_application::oidc::OidcError;
use ferrobox_application::manage_api_tokens::{
    CreateApiTokenError, ListApiTokensError, RevokeApiTokenError,
};
use ferrobox_application::manage_groups::GroupError;
use ferrobox_application::manage_users::{
    ChangeUserRoleError, CreateUserError, DeleteUserError, ListUsersError, ResetUserPasswordError,
};
use ferrobox_application::packaging::PackagingError;
use ferrobox_application::promote_package::PromoteError;
use ferrobox_application::publish_artifact::PublishArtifactError;
use ferrobox_application::quota::QuotaError;
use ferrobox_application::retention::RetentionError;
use ferrobox_application::search_packages::SearchPackagesError;
use ferrobox_application::update_alloy_members::UpdateAlloyMembersError;
use ferrobox_application::webhooks::ManageWebhookError;
use ferrobox_domain::quota::StorageQuotaError;
use ferrobox_domain::retention::RetentionPolicyError;
use ferrobox_ports::artifact_store::ArtifactStoreError;
use ferrobox_ports::group_store::GroupStoreError;
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
            | CreateRepositoryError::InvalidUpstream(_)
            | CreateRepositoryError::EmptyAlloy
            | CreateRepositoryError::MemberNotFound
            | CreateRepositoryError::MemberEcosystemMismatch
            | CreateRepositoryError::NestedAlloy => Self::BadRequest(err.to_string()),
            CreateRepositoryError::Persistence(RepositoryStoreError::DuplicateName(_)) => {
                Self::Conflict(err.to_string())
            }
            CreateRepositoryError::Persistence(RepositoryStoreError::Backend(_)) => {
                Self::Internal(err.to_string())
            }
        }
    }
}

impl From<UpdateAlloyMembersError> for ApiError {
    fn from(err: UpdateAlloyMembersError) -> Self {
        match &err {
            UpdateAlloyMembersError::NotFound(_) => Self::NotFound(err.to_string()),
            UpdateAlloyMembersError::NotAnAlloy
            | UpdateAlloyMembersError::EmptyAlloy
            | UpdateAlloyMembersError::MemberNotFound
            | UpdateAlloyMembersError::MemberEcosystemMismatch
            | UpdateAlloyMembersError::NestedAlloy => Self::BadRequest(err.to_string()),
            UpdateAlloyMembersError::Persistence(RepositoryStoreError::DuplicateName(_)) => {
                Self::Conflict(err.to_string())
            }
            UpdateAlloyMembersError::Persistence(RepositoryStoreError::Backend(_)) => {
                Self::Internal(err.to_string())
            }
        }
    }
}

impl From<PublishArtifactError> for ApiError {
    fn from(err: PublishArtifactError) -> Self {
        match err {
            PublishArtifactError::RepositoryNotFound(_) => Self::NotFound(err.to_string()),
            PublishArtifactError::ReadOnlyRepository => Self::BadRequest(err.to_string()),
            PublishArtifactError::Quota(inner) => inner.into(),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<PromoteError> for ApiError {
    fn from(err: PromoteError) -> Self {
        match err {
            PromoteError::SourceNotFound(_)
            | PromoteError::TargetNotFound(_)
            | PromoteError::ArtifactNotFound(_) => Self::NotFound(err.to_string()),
            PromoteError::SameRepository
            | PromoteError::SourceNotForge(_)
            | PromoteError::TargetNotForge(_)
            | PromoteError::EcosystemMismatch { .. }
            | PromoteError::MissingCoordinate
            | PromoteError::MissingArtifact
            | PromoteError::ArtifactRepositoryMismatch(_) => Self::BadRequest(err.to_string()),
            PromoteError::Quota(inner) => inner.into(),
            PromoteError::Packaging(inner) => inner.into(),
            PromoteError::Repository(_) | PromoteError::Artifact(_) | PromoteError::Storage(_) => {
                Self::Internal(err.to_string())
            }
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

impl From<ListRepositoryArtifactsError> for ApiError {
    fn from(err: ListRepositoryArtifactsError) -> Self {
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
            PackagingError::Quota(inner) => inner.into(),
            PackagingError::PolicyDenied(message) => Self::Forbidden(message),
            PackagingError::PackageNotFound(_)
            | PackagingError::VersionNotFound(_)
            | PackagingError::FileNotFound(_)
            | PackagingError::Upstream(ferrobox_ports::http_client::HttpClientError::Status {
                status: 404,
                ..
            }) => Self::NotFound(err.to_string()),
            PackagingError::Storage(_)
            | PackagingError::ArtifactPersistence(_)
            | PackagingError::IndexPersistence(_)
            | PackagingError::RepositoryPersistence(_)
            | PackagingError::ChecksumMismatch { .. }
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
            CreateUserError::InvalidPassword(_) => Self::BadRequest(err.to_string()),
            CreateUserError::Persistence(
                ferrobox_ports::user_store::UserStoreError::DuplicateUsername(_)
                | ferrobox_ports::user_store::UserStoreError::DuplicateEmail(_),
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

impl From<DeleteRepositoryError> for ApiError {
    fn from(err: DeleteRepositoryError) -> Self {
        match err {
            DeleteRepositoryError::NotFound(_) => Self::NotFound(err.to_string()),
            DeleteRepositoryError::RepositoryPersistence(_)
            | DeleteRepositoryError::ArtifactPersistence(_)
            | DeleteRepositoryError::IndexPersistence(_)
            | DeleteRepositoryError::Storage(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<DeleteArtifactError> for ApiError {
    fn from(err: DeleteArtifactError) -> Self {
        match err {
            DeleteArtifactError::RepositoryNotFound(_)
            | DeleteArtifactError::ArtifactNotFound(_, _) => Self::NotFound(err.to_string()),
            DeleteArtifactError::RepositoryPersistence(_)
            | DeleteArtifactError::ArtifactPersistence(_)
            | DeleteArtifactError::IndexPersistence(_)
            | DeleteArtifactError::Storage(_) => Self::Internal(err.to_string()),
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

impl From<ChangePasswordError> for ApiError {
    fn from(err: ChangePasswordError) -> Self {
        match err {
            ChangePasswordError::InvalidPassword(_)
            | ChangePasswordError::InvalidCurrentPassword => Self::BadRequest(err.to_string()),
            ChangePasswordError::NotFound => Self::NotFound(err.to_string()),
            ChangePasswordError::PasswordHashing(_) | ChangePasswordError::Persistence(_) => {
                Self::Internal(err.to_string())
            }
        }
    }
}

impl From<ResetUserPasswordError> for ApiError {
    fn from(err: ResetUserPasswordError) -> Self {
        match err {
            ResetUserPasswordError::InvalidPassword(_) => Self::BadRequest(err.to_string()),
            ResetUserPasswordError::CannotResetSelf => Self::Conflict(err.to_string()),
            ResetUserPasswordError::NotFound => Self::NotFound(err.to_string()),
            ResetUserPasswordError::PasswordHashing(_) | ResetUserPasswordError::Persistence(_) => {
                Self::Internal(err.to_string())
            }
        }
    }
}

impl From<ferrobox_domain::admission::AdmissionPolicyError> for ApiError {
    fn from(err: ferrobox_domain::admission::AdmissionPolicyError) -> Self {
        Self::BadRequest(err.to_string())
    }
}

impl From<AdmissionError> for ApiError {
    fn from(err: AdmissionError) -> Self {
        match err {
            AdmissionError::RepositoryNotFound(_) => Self::NotFound(err.to_string()),
            AdmissionError::AlloyRepository
            | AdmissionError::UnsupportedRepository
            | AdmissionError::InvalidPolicy(_)
            | AdmissionError::Store(
                ferrobox_ports::admission_store::AdmissionStoreError::MissingSchema,
            ) => Self::BadRequest(err.to_string()),
            AdmissionError::Store(_)
            | AdmissionError::Artifacts(_)
            | AdmissionError::Repositories(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<RetentionPolicyError> for ApiError {
    fn from(err: RetentionPolicyError) -> Self {
        Self::BadRequest(err.to_string())
    }
}

impl From<StorageQuotaError> for ApiError {
    fn from(err: StorageQuotaError) -> Self {
        Self::BadRequest(err.to_string())
    }
}

impl From<QuotaError> for ApiError {
    fn from(err: QuotaError) -> Self {
        match err {
            QuotaError::RepositoryNotFound(_) => Self::NotFound(err.to_string()),
            QuotaError::AlloyRepository
            | QuotaError::InvalidQuota(_)
            | QuotaError::MissingSchema => Self::BadRequest(err.to_string()),
            QuotaError::Exceeded { .. } => Self::Conflict(err.to_string()),
            QuotaError::Repositories(_) | QuotaError::Artifacts(_) | QuotaError::Policy(_) => {
                Self::Internal(err.to_string())
            }
        }
    }
}

impl From<RetentionError> for ApiError {
    fn from(err: RetentionError) -> Self {
        match err {
            RetentionError::RepositoryNotFound(_) => Self::NotFound(err.to_string()),
            RetentionError::AlloyRepository | RetentionError::InvalidPolicy(_) => {
                Self::BadRequest(err.to_string())
            }
            RetentionError::MissingSchema => Self::BadRequest(err.to_string()),
            RetentionError::Repositories(_)
            | RetentionError::Artifacts(_)
            | RetentionError::Index(_)
            | RetentionError::Assays(_)
            | RetentionError::Policy(_)
            | RetentionError::Storage(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<SearchPackagesError> for ApiError {
    fn from(err: SearchPackagesError) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<GroupError> for ApiError {
    fn from(err: GroupError) -> Self {
        match err {
            GroupError::GroupNotFound => Self::NotFound(err.to_string()),
            GroupError::UserNotFound
            | GroupError::RepositoryNotFound
            | GroupError::InvalidGroupRole => Self::BadRequest(err.to_string()),
            GroupError::Groups(GroupStoreError::DuplicateName(_)) => {
                Self::Conflict(err.to_string())
            }
            GroupError::Groups(GroupStoreError::Backend(_))
            | GroupError::Users(_)
            | GroupError::Repositories(_) => Self::Internal(err.to_string()),
        }
    }
}

impl From<OidcError> for ApiError {
    fn from(err: OidcError) -> Self {
        match err {
            OidcError::Disabled => Self::NotFound(err.to_string()),
            OidcError::InvalidState
            | OidcError::Provider(_)
            | OidcError::InvalidToken(_)
            | OidcError::MissingClaims
            | OidcError::InvalidUsername => Self::BadRequest(err.to_string()),
            OidcError::Http(_)
            | OidcError::Users(_)
            | OidcError::Groups(_)
            | OidcError::Tokens(_)
            | OidcError::PasswordHash => Self::Internal(err.to_string()),
        }
    }
}

impl From<AuditError> for ApiError {
    fn from(err: AuditError) -> Self {
        Self::Internal(err.to_string())
    }
}

impl From<ManageWebhookError> for ApiError {
    fn from(err: ManageWebhookError) -> Self {
        match err {
            ManageWebhookError::RepositoryNotFound(_) | ManageWebhookError::WebhookNotFound(_) => {
                Self::NotFound(err.to_string())
            }
            ManageWebhookError::WebhookMismatch
            | ManageWebhookError::AlloyRepository
            | ManageWebhookError::Invalid(_)
            | ManageWebhookError::Store(
                ferrobox_ports::webhook_store::WebhookStoreError::MissingSchema,
            ) => Self::BadRequest(err.to_string()),
            ManageWebhookError::Store(_) | ManageWebhookError::Repositories(_) => {
                Self::Internal(err.to_string())
            }
        }
    }
}
