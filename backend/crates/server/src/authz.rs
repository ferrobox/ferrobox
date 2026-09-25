//! Comprobaciones de autorización basadas en el rol del usuario y en
//! el acceso de grupos a repositorios.

use axum::http::HeaderMap;
use ferrobox_application::authenticate_token::AuthenticateTokenUseCase;
use ferrobox_application::manage_groups::GroupService;
use ferrobox_domain::group::RepositoryAccess;
use ferrobox_domain::ids::RepositoryId;
use ferrobox_domain::user::User;

use crate::auth_extract::{OCI_ANONYMOUS_TOKEN, extract_bearer_token};
use crate::error::ApiError;

/// Exige que el usuario pueda gestionar otros usuarios (`Admin`).
pub(crate) fn require_manage_users(user: &User) -> Result<(), ApiError> {
    if user.role().can_manage_users() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin role required to manage users".to_string(),
        ))
    }
}

/// Exige que el usuario pueda gestionar grupos (`Admin`).
pub(crate) fn require_manage_groups(user: &User) -> Result<(), ApiError> {
    if user.role().can_manage_users() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin role required to manage groups".to_string(),
        ))
    }
}

/// Exige que el usuario pueda crear repositorios y publicar artefactos
/// (`Admin` o `Developer`) a nivel de instancia.
pub(crate) fn require_write_artifacts(user: &User) -> Result<(), ApiError> {
    if user.role().can_write_artifacts() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "admin or developer role required to write artifacts".to_string(),
        ))
    }
}

/// Exige lectura del repositorio (rol de instancia o membresía de grupo).
pub(crate) async fn require_repo_read(
    groups: &GroupService,
    user: &User,
    repository_id: RepositoryId,
) -> Result<RepositoryAccess, ApiError> {
    groups
        .access_on(user, repository_id)
        .await?
        .ok_or_else(|| ApiError::Forbidden("you do not have access to this repository".to_string()))
}

/// Exige escritura en el repositorio.
pub(crate) async fn require_repo_write(
    groups: &GroupService,
    user: &User,
    repository_id: RepositoryId,
) -> Result<(), ApiError> {
    let access = require_repo_read(groups, user, repository_id).await?;
    if access.can_write() {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "write access to this repository is required".to_string(),
        ))
    }
}

/// Lectura pública de un registro: si el repositorio está restringido,
/// exige un token con acceso de lectura.
pub(crate) async fn require_public_repo_read(
    groups: &GroupService,
    authenticate: &AuthenticateTokenUseCase,
    headers: &HeaderMap,
    repository_id: RepositoryId,
) -> Result<(), ApiError> {
    if !groups.is_restricted(repository_id).await? {
        return Ok(());
    }

    let Some(user) = authenticate_from_headers(authenticate, headers).await? else {
        return Err(ApiError::Unauthorized(
            "authentication required to read this repository".to_string(),
        ));
    };
    require_repo_read(groups, &user, repository_id).await?;
    Ok(())
}

async fn authenticate_from_headers(
    authenticate: &AuthenticateTokenUseCase,
    headers: &HeaderMap,
) -> Result<Option<User>, ApiError> {
    let Some(secret) = extract_bearer_token(headers) else {
        return Ok(None);
    };
    if secret == OCI_ANONYMOUS_TOKEN {
        return Ok(None);
    }
    Ok(Some(authenticate.execute(&secret).await?.user))
}
